//! `mars/sdt/src/tools/netchecker_trafficmonitor.{h,cc}` — how much traffic a
//! diagnosis is allowed to cost.
//!
//! `tcpquery.cc` keeps one of these next to the socket it probes: it counts what
//! the probe sent and received, and `send_limit_check` / `recv_limit_check` say
//! when the budget is gone — a diagnosis is not allowed to cost the app more
//! traffic than the threshold, and the counters are kept per network, mobile
//! separately from wifi.
//!
//! Two things are the host's, since neither is this crate's to read:
//!
//! * which network the bytes went over — the C++ asks `comm::getNetInfo()`, and
//!   it asks twice per call (once for the sent bytes, once for the received
//!   ones): [`NetCheckTrafficMonitor::send_limit_check`] and
//!   [`NetCheckTrafficMonitor::recv_limit_check`] take that answer with the
//!   size, and ask once;
//! * the sizes themselves, which in the C++ come off a socket counter the host
//!   owns.
//!
//! Not ported: `__dumpDataSize()`, which is what the C++'s destructor logs —
//! there is no logger in this crate.

/// `ULONG_MAX` — the `wifiDataThreshold` of a monitor that was built without
/// one, i.e. one that does not limit wifi traffic at all.
pub const DEFAULT_WIFI_DATA_THRESHOLD: u64 = u64::MAX;

/// `NetCheckTrafficMonitor` — the traffic budget of one run of probes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetCheckTrafficMonitor {
    wifi_recv: u64,
    wifi_send: u64,
    mobile_recv: u64,
    mobile_send: u64,
    wifi_data_threshold: u64,
    mobile_data_threshold: u64,
    is_ignore_recv_data: bool,
}

impl NetCheckTrafficMonitor {
    /// `NetCheckTrafficMonitor(mobileDataThreshold, isIgnoreRecvData)` — the
    /// wifi threshold defaults to [`DEFAULT_WIFI_DATA_THRESHOLD`] in the C++,
    /// which is a budget wifi traffic cannot use up.
    pub fn new(mobile_data_threshold: u64, is_ignore_recv_data: bool) -> Self {
        Self::with_wifi_threshold(
            mobile_data_threshold,
            is_ignore_recv_data,
            DEFAULT_WIFI_DATA_THRESHOLD,
        )
    }

    /// `NetCheckTrafficMonitor(mobileDataThreshold, isIgnoreRecvData,
    /// wifiDataThreshold)`.
    pub fn with_wifi_threshold(
        mobile_data_threshold: u64,
        is_ignore_recv_data: bool,
        wifi_data_threshold: u64,
    ) -> Self {
        Self {
            wifi_recv: 0,
            wifi_send: 0,
            mobile_recv: 0,
            mobile_send: 0,
            wifi_data_threshold,
            mobile_data_threshold,
            is_ignore_recv_data,
        }
    }

    /// What was received over wifi.
    pub fn wifi_recv(&self) -> u64 {
        self.wifi_recv
    }

    /// What was sent over wifi.
    pub fn wifi_send(&self) -> u64 {
        self.wifi_send
    }

    /// What was received over mobile data.
    pub fn mobile_recv(&self) -> u64 {
        self.mobile_recv
    }

    /// What was sent over mobile data.
    pub fn mobile_send(&self) -> u64 {
        self.mobile_send
    }

    /// `sendLimitCheck(sendDataSize)` — `true`, the C++'s warning, when sending
    /// `send_data_size` more would pass a threshold. Nothing is counted then:
    /// the data the check refused is data that never went out.
    ///
    /// `mobile` is the `kMobile != getNetInfo()` of the C++.
    ///
    /// The C++ compares the size against **both** thresholds whatever network
    /// the bytes are going over, and so does this: on wifi, a size that would
    /// pass the mobile threshold is refused as well.
    pub fn send_limit_check(&mut self, send_data_size: u64, mobile: bool) -> bool {
        if self.wifi_send.saturating_add(send_data_size) > self.wifi_data_threshold
            || self.mobile_send.saturating_add(send_data_size) > self.mobile_data_threshold
        {
            return true;
        }
        self.data(send_data_size, 0, mobile);
        false
    }

    /// `recvLimitCheck(recvDataSize)` — `true` when what has come back has
    /// passed a threshold, which is only asked of a monitor built with
    /// `is_ignore_recv_data = false`.
    ///
    /// The received bytes are counted **before** the check, and they are counted
    /// whether the monitor looks at them or not: a monitor that ignores them for
    /// the limit still remembers them.
    pub fn recv_limit_check(&mut self, recv_data_size: u64, mobile: bool) -> bool {
        self.data(0, recv_data_size, mobile);
        if self.is_ignore_recv_data {
            return false;
        }
        self.wifi_send.saturating_add(self.wifi_recv) > self.wifi_data_threshold
            || self.mobile_send.saturating_add(self.mobile_recv) > self.mobile_data_threshold
    }

    /// `reset()` — the counters go back to nothing, and so do **both**
    /// thresholds: a monitor that has been reset refuses everything, since every
    /// size is more than `0`.
    pub fn reset(&mut self) {
        self.wifi_recv = 0;
        self.wifi_send = 0;
        self.mobile_recv = 0;
        self.mobile_send = 0;
        self.wifi_data_threshold = 0;
        self.mobile_data_threshold = 0;
    }

    /// `__data(sendDataSize, recvDataSize)` — one call's worth of traffic, on the
    /// network `mobile` says it went over. Nothing is counted for a call that
    /// neither sent nor received anything.
    fn data(&mut self, send_data_size: u64, recv_data_size: u64, mobile: bool) {
        if send_data_size > 0 || recv_data_size > 0 {
            if mobile {
                self.mobile_recv = self.mobile_recv.saturating_add(recv_data_size);
                self.mobile_send = self.mobile_send.saturating_add(send_data_size);
            } else {
                self.wifi_recv = self.wifi_recv.saturating_add(recv_data_size);
                self.wifi_send = self.wifi_send.saturating_add(send_data_size);
            }
        }
    }
}
