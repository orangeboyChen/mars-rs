//! `mars/stn/proto/longlink_packer.cc` — the long-link wire format.
//!
//! Every long-link package starts with `__STNetMsgXpHeader`, twenty packed
//! bytes of network-endian `uint32_t`:
//!
//! ```text
//! head_length | client_version | cmdid | seq | body_length
//! ```
//!
//! `longlink_pack` prepends it to the body; `longlink_unpack` reads it back and
//! answers one of three things — `Continue` (not enough bytes yet), `False`
//! (this is not a package of ours) or `Ok` with the cmdid, the seq, how long
//! the whole package is and the body.
//!
//! The C++ writes into an `AutoBuffer`; the port hands back a `Vec<u8>` and
//! takes `&[u8]`, which is the same bytes either way. `longlink_tracker` is a
//! pure C++ hook object (it carries nothing), so there is no counterpart here.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::config::MIN_HEART_INTERVAL;
use crate::Task;

/// Not enough bytes for a header yet: read more and call again.
pub const LONGLINK_UNPACK_CONTINUE: i32 = -2;
/// The bytes are a package, but not one of ours (a version mismatch, or a
/// package that is too big).
pub const LONGLINK_UNPACK_FALSE: i32 = -1;
/// A whole package.
pub const LONGLINK_UNPACK_OK: i32 = 0;
/// For HTTP/2, the end of a stream is a package boundary like any other.
pub const LONGLINK_UNPACK_STREAM_END: i32 = LONGLINK_UNPACK_OK;
/// … and a stream package is one the caller still has to read.
pub const LONGLINK_UNPACK_STREAM_PACKAGE: i32 = 1;

/// `NOOP_CMDID` — the heartbeat.
pub const NOOP_CMDID: u32 = 6;
/// `SIGNALKEEP_CMDID`.
pub const SIGNALKEEP_CMDID: u32 = 243;
/// `PUSH_DATA_TASKID` — a package with this task id is the server pushing.
pub const PUSH_DATA_TASKID: u32 = 0;

/// `__STNetMsgXpHeader`, `#pragma pack(1)`: five `uint32_t`, no padding.
pub const HEADER_LEN: usize = 20;
/// A package bigger than this is refused — the C++'s `1024 * 1024`.
pub const MAX_PACKAGE_LEN: usize = 1024 * 1024;

/// What `SetClientVersion` set: the version every package goes out with, and
/// the only version `longlink_unpack` accepts back.
static CLIENT_VERSION: AtomicU32 = AtomicU32::new(0);

/// `mars::stn::SetClientVersion` — what `StnLogic.setClientVersion` calls.
pub fn set_client_version(client_version: u32) {
    CLIENT_VERSION.store(client_version, Ordering::SeqCst);
}

/// The version the packages are stamped with.
pub fn client_version() -> u32 {
    CLIENT_VERSION.load(Ordering::SeqCst)
}

/// `PackerEncoderVersion` — the "mars2" encoder, which the C++ only reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PackerEncoderVersion {
    /// `kOld = 1`.
    #[default]
    Old = 1,
    /// `kNew = 2`.
    New = 2,
}

/// `__STNetMsgXpHeader`, in host order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StNetMsgXpHeader {
    /// How long the header is: `HEADER_LEN`, whatever the version.
    pub head_length: u32,
    /// `sg_client_version` at pack time.
    pub client_version: u32,
    /// Business identifier.
    pub cmdid: u32,
    /// Task id.
    pub seq: u32,
    /// How long the body is.
    pub body_length: u32,
}

impl StNetMsgXpHeader {
    /// The twenty bytes, network endian.
    pub fn to_bytes(self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        for (index, field) in [
            self.head_length,
            self.client_version,
            self.cmdid,
            self.seq,
            self.body_length,
        ]
        .into_iter()
        .enumerate()
        {
            out[index * 4..index * 4 + 4].copy_from_slice(&field.to_be_bytes());
        }
        out
    }

    /// Reads the twenty bytes back, the way `memcpy` into the packed struct
    /// does: the C++ then `ntohl`s each field.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut fields = [0u32; 5];
        for (index, field) in fields.iter_mut().enumerate() {
            let at = index * 4;
            if at + 4 > bytes.len() {
                break;
            }
            let mut be = [0u8; 4];
            be.copy_from_slice(&bytes[at..at + 4]);
            *field = u32::from_be_bytes(be);
        }
        Self {
            head_length: fields[0],
            client_version: fields[1],
            cmdid: fields[2],
            seq: fields[3],
            body_length: fields[4],
        }
    }
}

/// `longlink_pack(cmdid, seq, body, extension)` — the header followed by the
/// body.
///
/// The C++ also takes an `_extension` buffer; the default encoder writes
/// nothing from it, so this one leaves it out.
pub fn longlink_pack(cmdid: u32, seq: u32, body: &[u8]) -> Vec<u8> {
    let header = StNetMsgXpHeader {
        head_length: HEADER_LEN as u32,
        client_version: client_version(),
        cmdid,
        seq,
        body_length: body.len() as u32,
    };
    let mut packed = Vec::with_capacity(HEADER_LEN + body.len());
    packed.extend_from_slice(&header.to_bytes());
    packed.extend_from_slice(body);
    packed
}

/// What `longlink_unpack` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unpacked {
    /// `LONGLINK_UNPACK_CONTINUE` — read more bytes and try again.
    Continue,
    /// `LONGLINK_UNPACK_FALSE` — not a package of ours.
    False,
    /// `LONGLINK_UNPACK_OK` — a whole package.
    Package {
        /// Business identifier.
        cmdid: u32,
        /// Task id.
        seq: u32,
        /// How long the whole package is, header included.
        package_len: usize,
        /// The body: everything after the header.
        body: Vec<u8>,
    },
}

impl Unpacked {
    /// The `int` the C++ answers with.
    pub fn code(&self) -> i32 {
        match self {
            Self::Continue => LONGLINK_UNPACK_CONTINUE,
            Self::False => LONGLINK_UNPACK_FALSE,
            Self::Package { .. } => LONGLINK_UNPACK_OK,
        }
    }
}

/// `longlink_unpack` — the C++ answers an `int` and fills `_cmdid`, `_seq`,
/// `_package_len`, `_body` and `_extension` through references; the port hands
/// back the same four in one value.
///
/// `head_length` is taken from the stream and is **not** checked against
/// [`HEADER_LEN`]: like the C++ `__unpack_test`, which takes it as it comes, a
/// header that claims `0` is a package of `0` bytes — an answer of
/// [`Unpacked::Package`] whose `package_len` is `0`. That is what the C++
/// answers too, and `LongLink::__ReadWrite` is the only consumer it has, so the
/// port leaves it to the caller: whoever advances a stream by `package_len`
/// has to stop on a `0`, or it will never move — which is what the long link
/// does, ending its run on one.
pub fn longlink_unpack(packed: &[u8]) -> Unpacked {
    // `__unpack_test`
    if packed.len() < HEADER_LEN {
        return Unpacked::Continue;
    }
    let header = StNetMsgXpHeader::from_bytes(packed);

    if header.client_version != client_version() {
        return Unpacked::False;
    }
    let body_len = header.body_length as usize;
    let package_len = header.head_length as usize + body_len;

    if package_len > MAX_PACKAGE_LEN {
        return Unpacked::False;
    }
    if package_len > packed.len() {
        return Unpacked::Continue;
    }

    let body = packed[package_len - body_len..package_len].to_vec();
    Unpacked::Package {
        cmdid: header.cmdid,
        seq: header.seq,
        package_len,
        body,
    }
}

/// `LongLinkEncoder` — the `std::function`s the C++ lets the app replace, as
/// the methods they default to. `gDefaultLongLinkEncoder` is [`LongLinkEncoder`]
/// itself: nothing about it is per-connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LongLinkEncoder {
    /// `packer_encoder_version` — `kOld` until the app asks for `kNew`.
    pub packer_encoder_version: PackerEncoderVersion,
}

impl LongLinkEncoder {
    /// `LongLinkEncoder()` — `PackerEncoderVersion::kOld`, like the C++ static.
    pub fn new() -> Self {
        Self::default()
    }

    /// `SetEncoderVersion`.
    pub fn set_encoder_version(&mut self, version: i32) {
        self.packer_encoder_version = match version {
            2 => PackerEncoderVersion::New,
            _ => PackerEncoderVersion::Old,
        };
    }

    /// `longlink_noop_cmdid()`.
    pub fn noop_cmdid(&self) -> u32 {
        NOOP_CMDID
    }

    /// `signal_keep_cmdid()`.
    pub fn signal_keep_cmdid(&self) -> u32 {
        SIGNALKEEP_CMDID
    }

    /// `longlink_noop_interval()` — `0`, so the long link decides for itself.
    pub fn noop_interval(&self) -> u32 {
        0
    }

    /// `longlink_complexconnect_need_verify()` — `false`.
    pub fn complexconnect_need_verify(&self) -> bool {
        false
    }

    /// `longlink_noop_isresp(taskid, cmdid, recv_seq, body, extend)` — the
    /// heartbeat answer is the noop task with the noop cmdid.
    pub fn noop_isresp(&self, taskid: u32, cmdid: u32) -> bool {
        Task::NOOP_TASK_ID == taskid && NOOP_CMDID == cmdid
    }

    /// `longlink_ispush(cmdid, taskid, body, extend)` — the server pushes with
    /// task id `0`.
    pub fn is_push(&self, taskid: u32) -> bool {
        PUSH_DATA_TASKID == taskid
    }

    /// `longlink_identify_isresp(sent_seq, cmdid, recv_seq, body, extend)`.
    pub fn identify_isresp(&self, sent_seq: u32, recv_seq: u32) -> bool {
        sent_seq == recv_seq && 0 != sent_seq
    }

    /// The heartbeat interval the encoder starts the long link on: the
    /// short end of the range, which is what `SmartHeartbeat` begins with too.
    pub fn heart_interval(&self) -> u32 {
        MIN_HEART_INTERVAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sg_client_version` is process-wide, so the tests that set it take the
    /// crate's turn: [`crate::test_lock`].
    fn with_client_version<R>(version: u32, f: impl FnOnce() -> R) -> R {
        let guard = crate::test_lock();
        let previous = client_version();
        set_client_version(version);
        let result = f();
        set_client_version(previous);
        drop(guard);
        result
    }

    #[test]
    fn the_header_is_twenty_packed_bytes_of_network_endian() {
        let header = StNetMsgXpHeader {
            head_length: HEADER_LEN as u32,
            client_version: 0x0102_0304,
            cmdid: 7,
            seq: 9,
            body_length: 3,
        };
        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), HEADER_LEN);
        // the first field is big endian… and so is the last
        assert_eq!(&bytes[0..4], &20u32.to_be_bytes());
        assert_eq!(&bytes[4..8], &0x0102_0304u32.to_be_bytes());
        assert_eq!(&bytes[16..20], &3u32.to_be_bytes());

        assert_eq!(StNetMsgXpHeader::from_bytes(&bytes), header);
    }

    #[test]
    fn a_short_read_still_gives_the_fields_it_has() {
        // `memcpy` of a short buffer would be undefined in the C++; the port
        // reads what is there and leaves the rest at zero.
        let header = StNetMsgXpHeader::from_bytes(&[0, 0, 0, 20]);
        assert_eq!(header.head_length, 20);
        assert_eq!(header.body_length, 0);
    }

    #[test]
    fn pack_writes_the_header_then_the_body() {
        with_client_version(0x0102_0304, || {
            let packed = longlink_pack(11, 22, b"hello");
            assert_eq!(packed.len(), HEADER_LEN + 5);
            assert_eq!(&packed[HEADER_LEN..], b"hello");

            let header = StNetMsgXpHeader::from_bytes(&packed);
            assert_eq!(header.head_length, HEADER_LEN as u32);
            assert_eq!(header.client_version, 0x0102_0304);
            assert_eq!(header.cmdid, 11);
            assert_eq!(header.seq, 22);
            assert_eq!(header.body_length, 5);
        })
    }

    #[test]
    fn what_packs_unpacks() {
        with_client_version(200, || {
            let packed = longlink_pack(3, 4, b"the body");
            assert_eq!(
                longlink_unpack(&packed),
                Unpacked::Package {
                    cmdid: 3,
                    seq: 4,
                    package_len: HEADER_LEN + 8,
                    body: b"the body".to_vec(),
                }
            );
            assert_eq!(longlink_unpack(&packed).code(), LONGLINK_UNPACK_OK);
        })
    }

    #[test]
    fn an_empty_body_is_still_a_package() {
        with_client_version(0, || {
            let packed = longlink_pack(0, 0, &[]);
            assert_eq!(packed.len(), HEADER_LEN);
            assert_eq!(
                longlink_unpack(&packed),
                Unpacked::Package {
                    cmdid: 0,
                    seq: 0,
                    package_len: HEADER_LEN,
                    body: Vec::new(),
                }
            );
        })
    }

    #[test]
    fn a_short_buffer_asks_for_more() {
        assert_eq!(longlink_unpack(&[]), Unpacked::Continue);

        with_client_version(1, || {
            let packed = longlink_pack(1, 1, b"0123456789");
            // one byte short of the whole package
            let short = &packed[..packed.len() - 1];
            assert_eq!(longlink_unpack(short), Unpacked::Continue);
            assert_eq!(longlink_unpack(short).code(), LONGLINK_UNPACK_CONTINUE);
        })
    }

    #[test]
    fn a_package_of_another_version_is_refused() {
        with_client_version(300, || {
            let packed = longlink_pack(1, 1, b"x");
            set_client_version(301);
            assert_eq!(longlink_unpack(&packed), Unpacked::False);
            assert_eq!(longlink_unpack(&packed).code(), LONGLINK_UNPACK_FALSE);
        })
    }

    #[test]
    fn a_package_over_a_megabyte_is_refused() {
        with_client_version(0, || {
            // a header that claims more than the C++ allows
            let mut packed = longlink_pack(1, 1, &[]);
            let huge = (MAX_PACKAGE_LEN as u32 + 1).to_be_bytes();
            packed[16..20].copy_from_slice(&huge);
            assert_eq!(longlink_unpack(&packed), Unpacked::False);
        })
    }

    #[test]
    fn two_packages_in_one_buffer_are_read_one_after_the_other() {
        with_client_version(0, || {
            let mut stream = longlink_pack(1, 1, b"first");
            stream.extend_from_slice(&longlink_pack(2, 2, b"second"));

            let Unpacked::Package {
                package_len, body, ..
            } = longlink_unpack(&stream)
            else {
                panic!("the first package");
            };
            assert_eq!(body, b"first".to_vec());

            let Unpacked::Package { cmdid, body, .. } = longlink_unpack(&stream[package_len..])
            else {
                panic!("the second package");
            };
            assert_eq!(cmdid, 2);
            assert_eq!(body, b"second".to_vec());
        })
    }

    #[test]
    fn the_encoder_defaults_and_the_cmdids() {
        let mut encoder = LongLinkEncoder::new();
        assert_eq!(encoder.packer_encoder_version, PackerEncoderVersion::Old);
        encoder.set_encoder_version(2);
        assert_eq!(encoder.packer_encoder_version, PackerEncoderVersion::New);
        encoder.set_encoder_version(9);
        assert_eq!(encoder.packer_encoder_version, PackerEncoderVersion::Old);

        assert_eq!(encoder.noop_cmdid(), NOOP_CMDID);
        assert_eq!(encoder.signal_keep_cmdid(), SIGNALKEEP_CMDID);
        assert_eq!(encoder.noop_interval(), 0);
        assert!(!encoder.complexconnect_need_verify());
        assert_eq!(encoder.heart_interval(), MIN_HEART_INTERVAL);
    }

    #[test]
    fn the_encoder_tells_a_heartbeat_a_push_and_an_identify_apart() {
        let encoder = LongLinkEncoder::new();
        // the noop task with the noop cmdid
        assert!(encoder.noop_isresp(Task::NOOP_TASK_ID, NOOP_CMDID));
        assert!(!encoder.noop_isresp(Task::NOOP_TASK_ID, SIGNALKEEP_CMDID));
        assert!(!encoder.noop_isresp(1, NOOP_CMDID));

        // the server pushes with task id 0
        assert!(encoder.is_push(PUSH_DATA_TASKID));
        assert!(!encoder.is_push(1));

        // an identify answer carries the seq we sent
        assert!(encoder.identify_isresp(7, 7));
        assert!(!encoder.identify_isresp(7, 8));
        assert!(!encoder.identify_isresp(0, 0), "seq 0 is not an answer");
    }

    #[test]
    fn the_stream_constants_are_the_ones_the_http2_path_reads() {
        assert_eq!(LONGLINK_UNPACK_STREAM_END, LONGLINK_UNPACK_OK);
        assert_eq!(LONGLINK_UNPACK_STREAM_PACKAGE, 1);
    }
}
