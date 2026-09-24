//! `mars/stn/src/longlink_identify_checker.cc` — the check the long link makes
//! the app answer before it trusts the connection.
//!
//! The C++ asks `GetLonglinkIdentifyCheckBuffer` for a buffer, a hash and a
//! cmdid; the answer is *when* to send it, not whether. `kCheckNow` sends it at
//! once, `kCheckNext` asks again on the next connect and `kCheckNever` stops
//! asking. `OnLonglinkIdentifyResponse` is the app's verdict on the answer the
//! server sent back, and only a `true` there marks the connection checked.
//!
//! Both are app callbacks in `mars/stn/stn.h`, so they are callbacks here as
//! well — [`LongLinkIdentifyChecker::set_check_buffer`] and
//! [`LongLinkIdentifyChecker::set_on_response`] — and both are kept unset by
//! default, which is the one difference worth knowing: the C++ calls a global
//! that always exists, so an unset callback answers [`IdentifyMode::CheckNext`]
//! and `false`, i.e. "ask again later" and "not checked".
//!
//! What the C++ gets from its `Context` — `StnManager`, and through it the
//! channel the check belongs to — is what the callbacks carry here.

use crate::longlink::LongLinkEncoder;
use crate::task::Task;

/// `IdentifyMode` of `mars/stn/stn.h` — when the identify buffer goes out.
///
/// The comment above `GetLonglinkIdentifyCheckBuffer` in the C++ reads
/// `ECHECK_NOW = 0, ECHECK_NEVER = 1, ECHECK_NEXT = 2`, but the enum itself is
/// `kCheckNow = 0, kCheckNext, kCheckNever`, and that is the one the code
/// switches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IdentifyMode {
    /// `kCheckNow` — send it now.
    #[default]
    CheckNow = 0,
    /// `kCheckNext` — ask again on the next connect.
    CheckNext = 1,
    /// `kCheckNever` — stop asking.
    CheckNever = 2,
}

/// `GetLonglinkIdentifyCheckBuffer` — the app answers *when* the buffer goes
/// out, and — when it goes out now — the buffer, the hash of it, and the cmdid
/// to send it with.
///
/// The C++ hands the three of them out as references the app writes through,
/// which is what [`IdentifyBuffer`] replaces: the answer is one value, and an
/// app that has nothing to send says [`IdentifyBuffer::Next`] instead of filling
/// a buffer that is then thrown away.
pub type GetIdentifyCheckBuffer = dyn FnMut(&str, u32) -> IdentifyBuffer + Send;

/// What [`GetIdentifyCheckBuffer`] answers.
///
/// The hash is in every variant because the C++ fills it whatever it answers
/// and judges the response against it: a check that is put off still leaves the
/// hash behind ([`LongLinkIdentifyChecker::hash_code`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentifyBuffer {
    /// `kCheckNow` — send `buffer` with `cmdid`.
    Now {
        /// `identify_buffer`
        buffer: Vec<u8>,
        /// `buffer_hash`
        hash: Vec<u8>,
        /// `cmdid`
        cmdid: u32,
    },
    /// `kCheckNext` — ask again on the next connect.
    Next {
        /// `buffer_hash`
        hash: Vec<u8>,
    },
    /// `kCheckNever` — stop asking.
    Never {
        /// `buffer_hash`
        hash: Vec<u8>,
    },
}

impl IdentifyBuffer {
    /// `kCheckNow` — the buffer and the hash go out on `cmdid`.
    pub fn now(buffer: Vec<u8>, hash: Vec<u8>, cmdid: u32) -> Self {
        Self::Now {
            buffer,
            hash,
            cmdid,
        }
    }

    /// `kCheckNext`.
    pub fn next(hash: Vec<u8>) -> Self {
        Self::Next { hash }
    }

    /// `kCheckNever`.
    pub fn never(hash: Vec<u8>) -> Self {
        Self::Never { hash }
    }

    /// When it goes out — the [`IdentifyMode`] the C++ switches on.
    pub fn mode(&self) -> IdentifyMode {
        match self {
            Self::Now { .. } => IdentifyMode::CheckNow,
            Self::Next { .. } => IdentifyMode::CheckNext,
            Self::Never { .. } => IdentifyMode::CheckNever,
        }
    }

    /// The hash the response is judged against.
    pub fn hash(&self) -> &[u8] {
        match self {
            Self::Now { hash, .. } | Self::Next { hash } | Self::Never { hash } => hash,
        }
    }
}

/// `OnLonglinkIdentifyResponse` — the app's verdict on the answer, given the
/// response and the hash it handed out.
pub type OnIdentifyResponse = dyn FnMut(&str, &[u8], &[u8]) -> bool + Send;

/// `LongLinkIdentifyChecker`.
pub struct LongLinkIdentifyChecker {
    /// `channel_id_`
    channel_id: String,
    /// `is_minorlong_` — the cmdid is handed to the app with
    /// [`Task::MINOR_LONGLINK_CMD_MASK`] already in it.
    is_minor_long: bool,
    /// `has_checked_`
    has_checked: bool,
    /// `cmd_id_` — the cmdid of the buffer that went out.
    cmd_id: u32,
    /// `taskid_` — `SetID`, and what the response has to match.
    taskid: u32,
    /// `hash_code_buffer_` — what the app handed out and the response is judged
    /// against.
    hash_code: Vec<u8>,
    /// `encoder_` — `LongLinkEncoder::default()`, i.e. `gDefaultLongLinkEncoder`.
    encoder: LongLinkEncoder,
    check_buffer: Option<Box<GetIdentifyCheckBuffer>>,
    on_response: Option<Box<OnIdentifyResponse>>,
}

impl LongLinkIdentifyChecker {
    /// `LongLinkIdentifyChecker(encoder, channel_id, is_minorlong)`.
    pub fn new(channel_id: &str, is_minor_long: bool) -> Self {
        Self {
            channel_id: channel_id.to_owned(),
            is_minor_long,
            has_checked: false,
            cmd_id: 0,
            taskid: 0,
            hash_code: Vec::new(),
            encoder: LongLinkEncoder::default(),
            check_buffer: None,
            on_response: None,
        }
    }

    /// The encoder the response is judged with — `gDefaultLongLinkEncoder` by
    /// default, which is the one the C++ uses.
    pub fn set_encoder(&mut self, encoder: LongLinkEncoder) {
        self.encoder = encoder;
    }

    /// `GetLonglinkIdentifyCheckBuffer = …` — the app is handed the cmdid the
    /// buffer would go out with, and answers [`IdentifyBuffer`].
    pub fn set_check_buffer(
        &mut self,
        check_buffer: impl FnMut(&str, u32) -> IdentifyBuffer + Send + 'static,
    ) {
        self.check_buffer = Some(Box::new(check_buffer));
    }

    /// `OnLonglinkIdentifyResponse = …`.
    pub fn set_on_response(
        &mut self,
        on_response: impl FnMut(&str, &[u8], &[u8]) -> bool + Send + 'static,
    ) {
        self.on_response = Some(Box::new(on_response));
    }

    /// `has_checked_`
    pub fn has_checked(&self) -> bool {
        self.has_checked
    }

    /// `cmd_id_`
    pub fn cmd_id(&self) -> u32 {
        self.cmd_id
    }

    /// `taskid_`
    pub fn taskid(&self) -> u32 {
        self.taskid
    }

    /// `hash_code_buffer_`
    pub fn hash_code(&self) -> &[u8] {
        &self.hash_code
    }

    /// `channel_id_`
    pub fn channel_id(&self) -> &str {
        &self.channel_id
    }

    /// `GetIdentifyBuffer(buffer, cmdid)` — the buffer to send and the cmdid to
    /// send it with, which are the C++'s two out-parameters.
    ///
    /// [`None`] unless the app answered `kCheckNow`, in which case the buffer
    /// and the cmdid it answered with are what went out. `kCheckNever` marks the
    /// connection checked and `kCheckNext` leaves it to be asked again, and
    /// both answer [`None`]: the buffer is dropped, but the hash the app filled
    /// stays — the C++ resets `hash_code_buffer_` *before* the call, and the
    /// hash is what the response is judged against.
    ///
    /// The cmdid starts at `0` and, for a minor long link, gets
    /// [`Task::MINOR_LONGLINK_CMD_MASK`] OR'd into it **before** the app sees
    /// it — the order the C++ has, and one that matters: an app that answers
    /// with a cmdid of its own overwrites the mask. `__NoopReq` is the only
    /// caller and passes `0`.
    pub fn get_identify_buffer(&mut self) -> Option<(Vec<u8>, u32)> {
        if self.has_checked {
            return None;
        }

        let initial = if self.is_minor_long {
            Task::MINOR_LONGLINK_CMD_MASK
        } else {
            0
        };
        let answer = match self.check_buffer.as_mut() {
            Some(check_buffer) => check_buffer(&self.channel_id, initial),
            None => IdentifyBuffer::Next { hash: Vec::new() },
        };

        match answer {
            IdentifyBuffer::Never { hash } => {
                self.hash_code = hash;
                self.has_checked = true;
                None
            }
            IdentifyBuffer::Next { hash } => {
                self.hash_code = hash;
                self.has_checked = false;
                None
            }
            IdentifyBuffer::Now {
                buffer,
                hash,
                cmdid,
            } => {
                self.hash_code = hash;
                self.cmd_id = cmdid;
                Some((buffer, cmdid))
            }
        }
    }

    /// `SetID(taskid)`.
    pub fn set_id(&mut self, taskid: u32) {
        self.taskid = taskid;
    }

    /// `IsIdentifyResp(cmdid, taskid, buffer, extend)`.
    ///
    /// `encoder_.longlink_identify_isresp(taskid_, cmdid, taskid, …)`: the
    /// default encoder compares the two task ids and ignores the cmdid and the
    /// buffers, which is why they are not arguments here.
    pub fn is_identify_resp(&self, taskid: u32) -> bool {
        self.encoder.identify_isresp(self.taskid, taskid)
    }

    /// `OnIdentifyResp(buffer)` — the response the server sent back.
    ///
    /// `taskid_` is cleared whatever the app answers, and only a `true` marks
    /// the connection checked.
    pub fn on_identify_resp(&mut self, response: &[u8]) -> bool {
        let accepted = match self.on_response.as_mut() {
            Some(on_response) => on_response(&self.channel_id, response, &self.hash_code),
            None => false,
        };
        self.taskid = 0;
        if accepted {
            self.has_checked = true;
            return true;
        }
        false
    }

    /// `Reset()` — a new connection asks again.
    pub fn reset(&mut self) {
        self.has_checked = false;
        self.taskid = 0;
        self.cmd_id = 0;
        self.hash_code.clear();
    }
}

impl std::fmt::Debug for LongLinkIdentifyChecker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LongLinkIdentifyChecker")
            .field("channel_id", &self.channel_id)
            .field("is_minor_long", &self.is_minor_long)
            .field("has_checked", &self.has_checked)
            .field("cmd_id", &self.cmd_id)
            .field("taskid", &self.taskid)
            .field("hash_code", &self.hash_code)
            .finish()
    }
}

impl Default for LongLinkIdentifyChecker {
    fn default() -> Self {
        Self::new("", false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_that_is_never_made_ends_the_asking() {
        let mut checker = LongLinkIdentifyChecker::new("default", false);
        checker.set_check_buffer(|_channel, _cmdid| IdentifyBuffer::never(b"hash".to_vec()));

        assert!(checker.get_identify_buffer().is_none());
        assert!(checker.has_checked());
        // and once it is checked, the app is not asked again
        assert!(checker.get_identify_buffer().is_none());
        // the hash is what the response will be judged against, so it stays
        assert_eq!(checker.hash_code(), b"hash");
    }

    #[test]
    fn a_check_that_waits_for_the_next_connect_is_asked_again() {
        let mut checker = LongLinkIdentifyChecker::new("default", false);
        checker.set_check_buffer(|_channel, _cmdid| IdentifyBuffer::next(b"hash".to_vec()));

        assert!(checker.get_identify_buffer().is_none());
        assert!(!checker.has_checked());
        // the buffer the app filled does not go out, but the hash stays
        assert_eq!(checker.hash_code(), b"hash");
        assert_eq!(checker.cmd_id(), 0, "cmd_id_ is only set by kCheckNow");
    }

    #[test]
    fn a_check_that_is_made_now_goes_out_with_its_cmdid() {
        let buffer = IdentifyBuffer::now(b"identify".to_vec(), b"hash".to_vec(), 17);
        assert_eq!(buffer.mode(), IdentifyMode::CheckNow);
        assert_eq!(buffer.hash(), b"hash");
        assert_eq!(
            IdentifyBuffer::next(Vec::new()).mode(),
            IdentifyMode::CheckNext
        );
        assert_eq!(
            IdentifyBuffer::never(Vec::new()).mode(),
            IdentifyMode::CheckNever
        );

        let mut checker = LongLinkIdentifyChecker::new("default", false);
        checker.set_check_buffer(|_channel, _cmdid| {
            IdentifyBuffer::now(b"identify".to_vec(), b"hash".to_vec(), 17)
        });

        assert_eq!(
            checker.get_identify_buffer(),
            Some((b"identify".to_vec(), 17))
        );
        assert_eq!(checker.cmd_id(), 17);
        assert_eq!(checker.hash_code(), b"hash");
        assert!(!checker.has_checked(), "only the response marks it checked");
    }

    #[test]
    fn a_minor_long_link_hands_its_mask_to_the_app_before_it_writes() {
        let mut checker = LongLinkIdentifyChecker::new("minor", true);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(0u32));
        let recording = std::sync::Arc::clone(&seen);
        checker.set_check_buffer(move |channel, cmdid| {
            assert_eq!(channel, "minor");
            *recording.lock().unwrap_or_else(|p| p.into_inner()) = cmdid;
            IdentifyBuffer::now(Vec::new(), Vec::new(), cmdid)
        });

        // the mask is already in the cmdid the app is handed
        let (_buffer, cmdid) = checker.get_identify_buffer().unwrap();
        assert_eq!(
            *seen.lock().unwrap_or_else(|p| p.into_inner()),
            Task::MINOR_LONGLINK_CMD_MASK
        );
        assert_eq!(cmdid, Task::MINOR_LONGLINK_CMD_MASK);
        assert_eq!(checker.cmd_id(), Task::MINOR_LONGLINK_CMD_MASK);

        // ... and an app that writes one of its own overwrites the mask, which
        // is what the C++ does too: `_cmdid |= kMinorLonglinkCmdMask` runs
        // before the callback does
        let mut checker = LongLinkIdentifyChecker::new("minor", true);
        checker
            .set_check_buffer(|_channel, _cmdid| IdentifyBuffer::now(Vec::new(), Vec::new(), 17));
        assert_eq!(checker.get_identify_buffer().unwrap().1, 17);
    }

    #[test]
    fn the_response_is_the_one_that_was_asked_for() {
        let mut checker = LongLinkIdentifyChecker::new("default", false);
        checker.set_check_buffer(|_channel, _cmdid| {
            IdentifyBuffer::now(Vec::new(), b"hash".to_vec(), 17)
        });
        let _ = checker.get_identify_buffer();
        checker.set_id(42);

        // `longlink_identify_isresp`: the task ids have to match, and neither
        // may be 0
        assert!(checker.is_identify_resp(42));
        assert!(!checker.is_identify_resp(43));
        assert!(!checker.is_identify_resp(0));

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recording = std::sync::Arc::clone(&seen);
        checker.set_on_response(move |channel, response, hash| {
            recording
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((channel.to_owned(), response.to_vec(), hash.to_vec()));
            true
        });
        assert!(checker.on_identify_resp(b"resp"));
        assert!(checker.has_checked());
        assert_eq!(checker.taskid(), 0, "SetID is cleared by the response");
        assert_eq!(
            *seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec![("default".to_owned(), b"resp".to_vec(), b"hash".to_vec())]
        );
    }

    #[test]
    fn a_response_the_app_refuses_leaves_the_connection_unchecked() {
        let mut checker = LongLinkIdentifyChecker::new("default", false);
        checker.set_on_response(|_channel, _response, _hash| false);
        checker.set_id(42);

        assert!(!checker.on_identify_resp(b"resp"));
        assert!(!checker.has_checked());
        assert_eq!(checker.taskid(), 0, "the task id is cleared anyway");
    }

    #[test]
    fn reset_asks_again() {
        let mut checker = LongLinkIdentifyChecker::new("default", false);
        checker.set_check_buffer(|_channel, _cmdid| IdentifyBuffer::never(Vec::new()));
        let _ = checker.get_identify_buffer();
        assert!(checker.has_checked());

        checker.reset();
        assert!(!checker.has_checked());
        assert_eq!(checker.cmd_id(), 0);
        assert_eq!(checker.taskid(), 0);
        assert_eq!(checker.hash_code(), b"");
        // and the check is made again
        assert!(checker.get_identify_buffer().is_none());
        assert!(checker.has_checked());
    }

    #[test]
    fn without_the_app_the_answer_is_ask_again_and_not_checked() {
        let mut checker = LongLinkIdentifyChecker::default();
        assert!(checker.get_identify_buffer().is_none());
        assert!(!checker.has_checked());
        assert!(!checker.on_identify_resp(b"resp"));
        assert!(!checker.has_checked());
    }
}
