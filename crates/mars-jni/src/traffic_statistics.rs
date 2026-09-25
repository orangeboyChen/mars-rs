//! `mars/app/src/traffic_statistics.cc` — how much traffic the app has cost the
//! device, counted for the wifi and for the mobile network apart, and handed to
//! the app now and then.
//!
//! `TrafficStatistics` counts every byte STN sends and receives ([`TrafficStatistics::data`])
//! and, once the counters are worth reporting — [`DEFAULT_REPORT_TIMEOUT`]
//! since the last report, or more than [`DEFAULT_REPORT_SIZE_THRESHOLD`]
//! bytes — hands them to the callback the app set
//! ([`TrafficStatistics::set_report_flow`]) and starts over.
//! [`TrafficStatistics::flush`] reports what there is without waiting.
//!
//! Which network the bytes belong to is what [`crate::platform_comm::net_info_impl`]
//! answers: everything that is not the mobile network is the wifi one, which is
//! the C++'s `kMobile != getNetInfo()` — asked once here rather than once per
//! counter, the way the C++ asks it twice per call.
//!
//! No caller is left for this in mars, so this is the file as it stands. Two
//! things the C++ does are not ported: the `comm::Mutex` every method locks,
//! which a `&mut self` already is in Rust, and the `xassert2` of
//! `__ReportData`, so bytes counted with no callback set are dropped rather
//! than asserted about.

use mars_comm::tickcount::gettickcount;

use crate::platform_comm::{net_info_impl, NetInfo};

/// How often the counters are reported, when they are not big enough to be
/// reported before that: the C++'s `10 * 1000`, ten seconds.
pub const DEFAULT_REPORT_TIMEOUT: u64 = 10 * 1000;
/// How many bytes there have to be for the counters to be reported before the
/// timeout: the C++'s `10 * 1024`, ten kilobytes.
pub const DEFAULT_REPORT_SIZE_THRESHOLD: u32 = 10 * 1024;

/// `func_report_flow_` — who the counters are reported to: `wifi_recv,
/// wifi_send, mobile_recv, mobile_send`, in that order.
pub type ReportFlow = Box<dyn Fn(u32, u32, u32, u32) + Send>;

/// `mars::app::TrafficStatistics` — the wifi and the mobile counters of the
/// traffic the app has cost.
///
/// The callback is the app's own: `func_report_flow_(wifi_recv, wifi_send,
/// mobile_recv, mobile_send)` in that order, which is the order the C++ declares
/// them in and not the order it counts them in.
pub struct TrafficStatistics {
    report_timeout: u64,
    report_size_threshold: u32,
    report_flow: Option<ReportFlow>,
    wifi_recv_data_size: u32,
    wifi_send_data_size: u32,
    mobile_recv_data_size: u32,
    mobile_send_data_size: u32,
    last_report_time: u64,
}

impl TrafficStatistics {
    /// The C++'s default-constructed `TrafficStatistics`: ten seconds and ten
    /// kilobytes, counting from now.
    pub fn new() -> Self {
        Self::with_report_at(
            DEFAULT_REPORT_TIMEOUT,
            DEFAULT_REPORT_SIZE_THRESHOLD,
            gettickcount(),
        )
    }

    /// `TrafficStatistics(unsigned long _report_tmo, unsigned int _report_size_threshold)`
    /// — counting from now.
    pub fn with_report(report_timeout: u64, report_size_threshold: u32) -> Self {
        Self::with_report_at(report_timeout, report_size_threshold, gettickcount())
    }

    /// … counting from `now`, which is what the tests drive: the timeout is the
    /// only thing in here that reads the clock.
    pub fn with_report_at(report_timeout: u64, report_size_threshold: u32, now: u64) -> Self {
        TrafficStatistics {
            report_timeout,
            report_size_threshold,
            report_flow: None,
            wifi_recv_data_size: 0,
            wifi_send_data_size: 0,
            mobile_recv_data_size: 0,
            mobile_send_data_size: 0,
            last_report_time: now,
        }
    }

    /// `SetCallback` — who the counters are reported to.
    ///
    /// The C++ asserts the callback is set once (`xassert2(!func_report_flow_)`);
    /// here the second one is the one that answers.
    pub fn set_report_flow(&mut self, report_flow: impl Fn(u32, u32, u32, u32) + Send + 'static) {
        self.report_flow = Some(Box::new(report_flow));
    }

    /// `Data` — count `_send` bytes sent and `_recv` bytes received, and report
    /// if it is time.
    pub fn data(&mut self, send: u32, recv: u32) {
        self.data_at(send, recv, gettickcount())
    }

    /// … with the tick count handed over, which is what makes the timeout
    /// testable without waiting for it.
    pub fn data_at(&mut self, send: u32, recv: u32, now: u64) {
        if 0 < send || 0 < recv {
            let mobile = NetInfo::Mobile == net_info_impl();

            if mobile {
                self.mobile_recv_data_size = self.mobile_recv_data_size.wrapping_add(recv);
                self.mobile_send_data_size = self.mobile_send_data_size.wrapping_add(send);
            } else {
                self.wifi_recv_data_size = self.wifi_recv_data_size.wrapping_add(recv);
                self.wifi_send_data_size = self.wifi_send_data_size.wrapping_add(send);
            }
        }

        if self.is_should_report(now) {
            self.report_data(now);
        }
    }

    /// `Flush` — report what there is, timeout or no timeout.
    pub fn flush(&mut self) {
        self.flush_at(gettickcount())
    }

    /// … with the tick count handed over.
    pub fn flush_at(&mut self, now: u64) {
        self.report_data(now);
    }

    /// `__ReportData` — hand the counters over and start over.
    ///
    /// With no callback set the bytes are dropped, which is what the C++ does
    /// too once its `xassert2` has complained.
    fn report_data(&mut self, now: u64) {
        if let Some(report_flow) = &self.report_flow {
            if 0 < self.wifi_recv_data_size
                || 0 < self.wifi_send_data_size
                || 0 < self.mobile_recv_data_size
                || 0 < self.mobile_send_data_size
            {
                report_flow(
                    self.wifi_recv_data_size,
                    self.wifi_send_data_size,
                    self.mobile_recv_data_size,
                    self.mobile_send_data_size,
                );
            }
        }

        self.wifi_recv_data_size = 0;
        self.wifi_send_data_size = 0;
        self.mobile_recv_data_size = 0;
        self.mobile_send_data_size = 0;
        self.last_report_time = now;
    }

    /// `__IsShouldReport` — ten seconds since the last report, or ten kilobytes
    /// of counters.
    ///
    /// The C++ adds the four `unsigned int` counters up as they stand; this adds
    /// them as `u64`, so four counters near `u32::MAX` do not wrap into a
    /// threshold nobody has reached.
    fn is_should_report(&self, now: u64) -> bool {
        now.saturating_sub(self.last_report_time) > self.report_timeout
            || u64::from(self.wifi_recv_data_size)
                + u64::from(self.wifi_send_data_size)
                + u64::from(self.mobile_recv_data_size)
                + u64::from(self.mobile_send_data_size)
                > u64::from(self.report_size_threshold)
    }
}

impl Default for TrafficStatistics {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TrafficStatistics {
    /// The C++ has no `operator<<` for this; the callback it holds is not
    /// something to print anyway.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrafficStatistics")
            .field("report_timeout", &self.report_timeout)
            .field("report_size_threshold", &self.report_size_threshold)
            .field("wifi_recv", &self.wifi_recv_data_size)
            .field("wifi_send", &self.wifi_send_data_size)
            .field("mobile_recv", &self.mobile_recv_data_size)
            .field("mobile_send", &self.mobile_send_data_size)
            .field("last_report_time", &self.last_report_time)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use mars_comm::tickcount::gettickcount;

    use super::*;
    use crate::platform_comm::set_net_info_impl;

    /// The network the device is on is one value for the whole crate, so the
    /// tests that move it take the crate's lock and put it back.
    fn on_network<R>(net_info: NetInfo, f: impl FnOnce() -> R) -> R {
        let guard = crate::test_lock();
        set_net_info_impl(net_info.as_i32());
        let result = f();
        drop(guard);
        result
    }

    /// The four counters as the callback was handed them.
    type Reported = Arc<Mutex<Vec<(u32, u32, u32, u32)>>>;

    /// What the callback was handed, in the order it was handed it.
    fn reported() -> Reported {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn counting(reported: &Reported) -> TrafficStatistics {
        let sink = Arc::clone(reported);
        let mut statistics = TrafficStatistics::with_report_at(10_000, 10 * 1024, 1_000);
        statistics.set_report_flow(move |wifi_recv, wifi_send, mobile_recv, mobile_send| {
            sink.lock()
                .unwrap()
                .push((wifi_recv, wifi_send, mobile_recv, mobile_send));
        });
        statistics
    }

    #[test]
    fn bytes_are_counted_for_the_network_the_device_is_on() {
        // `kMobile != getNetInfo()` — everything that is not the mobile network
        // is the wifi one, and `NoNet` is not the mobile network either
        let reported = reported();
        let mut statistics = counting(&reported);

        on_network(NetInfo::Wifi, || {
            statistics.data_at(30, 40, 1_001);
            statistics.data_at(5, 0, 1_002);
        });
        on_network(NetInfo::Mobile, || statistics.data_at(7, 9, 1_003));
        on_network(NetInfo::NoNet, || statistics.data_at(1, 2, 1_004));

        statistics.flush_at(1_005);
        assert_eq!(
            *reported.lock().unwrap(),
            vec![(42, 36, 9, 7)],
            "wifi_recv, wifi_send, mobile_recv, mobile_send"
        );
    }

    #[test]
    fn nothing_is_reported_while_there_is_nothing_to_report() {
        // `0 < _send || 0 < _recv` guards the counting, and `0` of anything is
        // not worth a report either
        let reported = reported();
        let mut statistics = counting(&reported);

        statistics.data_at(0, 0, 1_001);
        statistics.flush_at(1_002);
        assert!(reported.lock().unwrap().is_empty());

        // a report starts the counters over, so what was reported is not
        // reported twice
        statistics.data_at(1, 1, 1_003);
        statistics.flush_at(1_004);
        statistics.flush_at(1_005);
        assert_eq!(*reported.lock().unwrap(), vec![(1, 1, 0, 0)]);
    }

    #[test]
    fn ten_kilobytes_are_reported_before_the_ten_seconds_are_up() {
        let reported = reported();
        let mut statistics = counting(&reported);

        // on the threshold: counted, not reported — `>` and not `>=`
        statistics.data_at(10 * 1024, 0, 1_001);
        assert!(reported.lock().unwrap().is_empty());

        // and the byte that crosses it
        statistics.data_at(1, 0, 1_002);
        assert_eq!(*reported.lock().unwrap(), vec![(0, 10 * 1024 + 1, 0, 0)]);

        // the threshold is the four counters added up, not any one of them
        statistics.data_at(5 * 1024, 0, 1_003);
        statistics.data_at(0, 5 * 1024 + 1, 1_004);
        assert_eq!(reported.lock().unwrap().len(), 2);
    }

    #[test]
    fn the_timeout_reports_what_there_is_however_little() {
        // `gettickcount() - last_report_time_ > report_timeout_`: one
        // millisecond past the ten seconds, not on it
        let reported = reported();
        let mut statistics = counting(&reported);

        statistics.data_at(1, 0, 11_000);
        assert!(reported.lock().unwrap().is_empty());

        statistics.data_at(0, 1, 11_001);
        assert_eq!(*reported.lock().unwrap(), vec![(1, 1, 0, 0)]);

        // … and the clock moving back under it is no report either
        statistics.data_at(1, 0, 0);
        assert_eq!(reported.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_flush_reports_without_waiting_for_either() {
        let reported = reported();
        let mut statistics = counting(&reported);

        statistics.data_at(1, 2, 1_001);
        statistics.flush_at(1_002);
        assert_eq!(*reported.lock().unwrap(), vec![(2, 1, 0, 0)]);

        // `flush` is the same thing with the clock read here
        on_network(NetInfo::Wifi, || statistics.data(3, 4));
        statistics.flush();
        assert_eq!(reported.lock().unwrap().len(), 2);
    }

    #[test]
    fn bytes_counted_with_nobody_to_report_them_to_are_dropped() {
        // the C++ asserts here (`xassert2(false, ...)`); the port drops them and
        // starts over like it does after a report
        let mut statistics = TrafficStatistics::with_report_at(10_000, 1, 1_000);
        on_network(NetInfo::Wifi, || statistics.data_at(1, 1, 1_001));
        statistics.flush_at(1_002);

        // … and a callback set later is not told about them
        let reported = reported();
        statistics.set_report_flow({
            let sink = Arc::clone(&reported);
            move |wifi_recv, wifi_send, mobile_recv, mobile_send| {
                sink.lock()
                    .unwrap()
                    .push((wifi_recv, wifi_send, mobile_recv, mobile_send));
            }
        });
        statistics.flush_at(1_003);
        assert!(reported.lock().unwrap().is_empty());

        // what it is told about is what is counted from then on
        on_network(NetInfo::Wifi, || statistics.data_at(1, 1, 1_004));
        assert_eq!(*reported.lock().unwrap(), vec![(1, 1, 0, 0)]);

        // a second callback is the one that answers, where the C++ asserts there
        // was not one already
        statistics.set_report_flow(|_, _, _, _| {});
        on_network(NetInfo::Wifi, || statistics.data_at(1, 1, 1_005));
        statistics.flush_at(1_006);
        assert_eq!(reported.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_counter_that_does_not_fit_in_thirty_two_bits_wraps() {
        // the C++ adds into an `unsigned int`, and this threshold is the
        // biggest one there is, so only the timeout can reach a report
        let reported = reported();
        let mut statistics = TrafficStatistics::with_report_at(10_000, u32::MAX, 1_000);
        statistics.set_report_flow({
            let sink = Arc::clone(&reported);
            move |wifi_recv, wifi_send, mobile_recv, mobile_send| {
                sink.lock()
                    .unwrap()
                    .push((wifi_recv, wifi_send, mobile_recv, mobile_send));
            }
        });

        statistics.data_at(u32::MAX - 5, 0, 1_001);
        assert!(
            reported.lock().unwrap().is_empty(),
            "a counter one byte short of wrapping is not a report yet"
        );

        // ten bytes more wrap it round to four, and the timeout reports that
        statistics.data_at(10, 0, 11_001);
        assert_eq!(*reported.lock().unwrap(), vec![(0, 4, 0, 0)]);

        // a counter that wrapped to nothing at all is not reported even on a
        // flush: the C++ only calls the callback when one of the four is not
        // zero
        statistics.data_at(u32::MAX, 0, 11_002);
        statistics.data_at(1, 0, 11_003);
        statistics.flush_at(11_004);
        assert_eq!(reported.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_statistics_counts_from_the_clock_it_was_given() {
        // `new()` and `with_report()` read the clock themselves, so the timeout
        // is ten seconds from now and not from a tick count of zero
        let mut statistics = TrafficStatistics::new();
        statistics.data_at(1, 1, 0);
        statistics.flush_at(0);
        assert!(format!("{statistics:?}").contains("TrafficStatistics"));

        let mut statistics = TrafficStatistics::with_report(10_000, 10 * 1024);
        statistics.data(1, 1);
        assert!(gettickcount() >= statistics.last_report_time);

        // `default()` is `new()`
        let statistics = TrafficStatistics::default();
        assert!(statistics.last_report_time <= gettickcount());
    }
}
