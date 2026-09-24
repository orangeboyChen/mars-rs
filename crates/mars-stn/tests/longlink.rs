//! `mars/stn/proto/longlink_packer.cc`, driven the way the long link drives
//! it: one stream of bytes in, one package at a time out.

use std::sync::{Mutex, MutexGuard, OnceLock};

use mars_stn::longlink::{
    client_version, longlink_pack, longlink_unpack, set_client_version, LongLinkEncoder, Unpacked,
    HEADER_LEN, LONGLINK_UNPACK_CONTINUE, LONGLINK_UNPACK_FALSE, LONGLINK_UNPACK_OK,
    MAX_PACKAGE_LEN, NOOP_CMDID, PUSH_DATA_TASKID, SIGNALKEEP_CMDID,
};
use mars_stn::Task;

/// `sg_client_version` is process-wide, so the tests that set it take it in
/// turn.
fn versions() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Reads the stream the way `LongLink::__ReadWrite` does: unpack, skip what
/// was read, stop when the rest is not a whole package yet.
fn drain(stream: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let mut packages = Vec::new();
    let mut rest = stream;
    loop {
        match longlink_unpack(rest) {
            Unpacked::Package {
                cmdid,
                seq,
                package_len,
                body,
            } => {
                packages.push((cmdid, seq, body));
                rest = &rest[package_len..];
                if rest.is_empty() {
                    return packages;
                }
            }
            // a package that has not arrived yet: wait for more bytes
            Unpacked::Continue | Unpacked::False => return packages,
        }
    }
}

#[test]
fn a_stream_carries_the_packages_it_was_packed_from() {
    let guard = versions();
    set_client_version(0x0203_0405);
    let mut stream = longlink_pack(NOOP_CMDID, Task::NOOP_TASK_ID, b"");
    stream.extend_from_slice(&longlink_pack(1, 101, b"a task"));
    stream.extend_from_slice(&longlink_pack(SIGNALKEEP_CMDID, 102, b"keep"));

    assert_eq!(
        drain(&stream),
        vec![
            (NOOP_CMDID, Task::NOOP_TASK_ID, Vec::new()),
            (1, 101, b"a task".to_vec()),
            (SIGNALKEEP_CMDID, 102, b"keep".to_vec()),
        ]
    );

    set_client_version(0);
    drop(guard);
}

#[test]
fn a_package_that_has_not_arrived_yet_is_read_again() {
    let guard = versions();
    set_client_version(7);
    let stream = longlink_pack(1, 2, b"0123456789");

    // nothing is complete until the last byte is there
    for at in 0..stream.len() - 1 {
        assert_eq!(
            longlink_unpack(&stream[..at]).code(),
            LONGLINK_UNPACK_CONTINUE,
            "{at} bytes"
        );
    }
    assert_eq!(longlink_unpack(&stream).code(), LONGLINK_UNPACK_OK);

    // and a stream that ends in the middle of a package keeps the first one
    let mut broken = stream.clone();
    broken.extend_from_slice(&longlink_pack(3, 4, b"tail")[..HEADER_LEN + 2]);
    assert_eq!(drain(&broken).len(), 1);

    set_client_version(0);
    drop(guard);
}

#[test]
fn what_the_long_link_asks_the_encoder() {
    let encoder = LongLinkEncoder::new();

    // the noop it sends, and the answer it accepts
    assert_eq!(encoder.noop_cmdid(), NOOP_CMDID);
    assert!(encoder.noop_isresp(Task::NOOP_TASK_ID, NOOP_CMDID));
    assert!(!encoder.noop_isresp(Task::NOOP_TASK_ID, SIGNALKEEP_CMDID));

    // a package with no task id is the server pushing
    assert!(encoder.is_push(PUSH_DATA_TASKID));
    assert!(!encoder.is_push(101));

    // and an identify answer is the seq that went out
    assert!(encoder.identify_isresp(11, 11));
    assert!(!encoder.identify_isresp(11, 12));

    // the interval it starts the long link on is the short end of the range
    assert_eq!(
        encoder.heart_interval(),
        mars_stn::config::MIN_HEART_INTERVAL
    );
}

#[test]
fn the_version_the_host_set_is_the_one_the_packages_carry() {
    let guard = versions();
    set_client_version(0x0102_0304);
    let packed = longlink_pack(1, 1, b"v");
    set_client_version(0);
    drop(guard);

    // the header is `head_length | client_version | cmdid | seq | body_length`
    assert_eq!(&packed[4..8], &0x0102_0304u32.to_be_bytes());
    assert_eq!(&packed[0..4], &(HEADER_LEN as u32).to_be_bytes());
    assert_eq!(client_version(), 0, "the tests leave it as they found it");
}

#[test]
fn a_package_that_is_not_ours_is_refused() {
    let guard = versions();
    set_client_version(1);
    let packed = longlink_pack(1, 1, b"x");

    // of another version…
    set_client_version(2);
    assert_eq!(longlink_unpack(&packed).code(), LONGLINK_UNPACK_FALSE);

    // … or bigger than the C++ allows
    set_client_version(1);
    let mut huge = longlink_pack(1, 1, &[]);
    huge[16..20].copy_from_slice(&(MAX_PACKAGE_LEN as u32 + 1).to_be_bytes());
    assert_eq!(longlink_unpack(&huge).code(), LONGLINK_UNPACK_FALSE);

    set_client_version(0);
    drop(guard);
}
