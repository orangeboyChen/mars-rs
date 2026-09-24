//! `mars/stn/src/frequency_limit.cc` — the avalanche table.
//!
//! The tick count is passed explicitly ([`FrequencyLimit::check_at`]) so that
//! the hourly sweep can be driven without waiting an hour; the expectations are
//! the ones of the C++, which passes while `count <= RECORD_INTERCEPT_COUNT`.

use mars_stn::frequency_limit::{
    adler32, FrequencyLimit, MAX_RECORD_COUNT, NOT_CLEAR_INTERCEPT_COUNT_RETRY,
    RECORD_INTERCEPT_COUNT,
};
use mars_stn::Task;

const T0: u64 = 1_000_000;
const HOUR: u64 = 60 * 60 * 1000;

fn task() -> Task {
    Task::new(1, 1)
}

#[test]
fn adler32_matches_the_reference_implementation() {
    // `mars/comm/adler32.c` is the Mark Adler reference implementation, which
    // is what zlib ships; these are zlib's values.
    assert_eq!(adler32(b""), 0x0000_0001);
    assert_eq!(adler32(b"a"), 0x0062_0062);
    assert_eq!(adler32(b"abc"), 0x024d_0127);
    assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    assert_eq!(
        adler32(b"The quick brown fox jumps over the lazy dog"),
        0x5bdc_0fda
    );
    assert_eq!(adler32(&(0..=255u8).collect::<Vec<u8>>()), 0xadf6_7f81);
    assert_eq!(adler32(&b"mars".repeat(100)), 0x17db_a9ed);

    // the hash the table keys on, and the two ends of the sum it is built from
    let a = 1u32;
    assert_eq!(adler32(b""), a);
    assert_ne!(adler32(b"a"), adler32(b"b"));
}

#[test]
fn a_task_that_does_not_limit_frequency_is_never_recorded() {
    let mut limit = FrequencyLimit::new_at(T0);
    let mut task = task();
    task.limit_frequency = false;

    for i in 0..200 {
        assert_eq!(limit.check_at(&task, b"anything", T0 + i), (true, 0));
    }
    assert!(limit.records().is_empty());
}

#[test]
fn an_unknown_body_is_recorded_and_the_second_send_reports_the_span() {
    let mut limit = FrequencyLimit::new_at(T0);
    let task = task();

    assert_eq!(limit.check_at(&task, b"body", T0), (true, 0));
    assert_eq!(limit.records().len(), 1);
    assert_eq!(limit.records()[0].hash, adler32(b"body"));
    assert_eq!(limit.records()[0].count, 1);
    assert_eq!(limit.records()[0].last_update, T0);

    // the C++ reports how long ago the same body went out
    assert_eq!(limit.check_at(&task, b"body", T0 + 1500), (true, 1500));
    assert_eq!(limit.records()[0].count, 2);
    assert_eq!(limit.records()[0].last_update, T0 + 1500);

    // another body gets its own record
    assert_eq!(limit.check_at(&task, b"other", T0 + 1500), (true, 0));
    assert_eq!(limit.records().len(), 2);
}

#[test]
fn one_body_may_go_out_105_times_and_then_the_gate_closes() {
    let mut limit = FrequencyLimit::new_at(T0);
    let task = task();

    assert_eq!(limit.check_at(&task, b"avalanche", T0), (true, 0));
    for send in 2..=RECORD_INTERCEPT_COUNT as u64 {
        assert!(
            limit.check_at(&task, b"avalanche", T0 + send).0,
            "send {send} of {RECORD_INTERCEPT_COUNT} was refused"
        );
    }
    assert_eq!(limit.records()[0].count, RECORD_INTERCEPT_COUNT);
    assert_eq!(
        limit.check_at(&task, b"avalanche", T0 + 1000),
        (false, 1000 - 105)
    );

    // the record stays refused once it is over the count
    assert!(!limit.check_at(&task, b"avalanche", T0 + 2000).0);
}

#[test]
fn the_table_holds_thirty_bodies_and_drops_the_one_touched_longest_ago() {
    let mut limit = FrequencyLimit::new_at(T0);
    let task = task();

    for i in 0..MAX_RECORD_COUNT as u64 {
        let body = format!("body-{i}");
        assert!(limit.check_at(&task, body.as_bytes(), T0 + i).0);
    }
    assert_eq!(limit.records().len(), MAX_RECORD_COUNT);

    // one more: the oldest record is evicted to make room
    let body = format!("body-{MAX_RECORD_COUNT}");
    assert!(limit.check_at(&task, body.as_bytes(), T0 + 1000).0);
    assert_eq!(limit.records().len(), MAX_RECORD_COUNT);
    assert!(!limit.records().iter().any(|r| r.hash == adler32(b"body-0")));
    assert!(limit
        .records()
        .iter()
        .any(|r| r.hash == adler32(b"body-30")));
}

#[test]
fn the_hourly_sweep_keeps_the_hot_records_and_caps_their_count() {
    let mut limit = FrequencyLimit::new_at(T0);
    let task = task();

    // 100 sends of "hot", the last one five minutes before the sweep
    for send in 0..99 {
        assert!(limit.check_at(&task, b"hot", T0 + send).0);
    }
    assert!(limit.check_at(&task, b"hot", T0 + 55 * 60 * 1000).0);
    assert_eq!(limit.records()[0].count, 100);

    // a cold record: sent once, an hour ago
    assert!(limit.check_at(&task, b"cold", T0 + 1).0);
    assert_eq!(limit.records().len(), 2);

    assert!(limit.check_at(&task, b"fresh", T0 + HOUR).0);
    assert_eq!(limit.last_clear(), T0 + HOUR);

    // "cold" is gone; "hot" is kept with its count lowered to the retry count
    let records = limit.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].hash, adler32(b"hot"));
    assert_eq!(records[0].count, NOT_CLEAR_INTERCEPT_COUNT_RETRY);
    assert_eq!(records[1].hash, adler32(b"fresh"));

    // an hour later even the hot record has gone cold
    assert!(limit.check_at(&task, b"later", T0 + 2 * HOUR).0);
    assert_eq!(limit.records().len(), 1);
    assert_eq!(limit.records()[0].hash, adler32(b"later"));
}

#[test]
fn the_gate_runs_against_the_real_clock_too() {
    let mut limit = FrequencyLimit::default();
    let mut task = task();
    task.limit_frequency = false;
    assert_eq!(limit.check(&task, b"anything"), (true, 0));

    let mut limited = FrequencyLimit::new();
    task.limit_frequency = true;
    assert!(limited.check(&task, b"anything").0);
    assert!(limited.check(&task, b"anything").0);
    assert!(!limited.records().is_empty());
}
