//! `tickcount_t` / `gettickcount()`.

use marsrs_comm::{TickCount, TickCountDiff};

#[test]
fn the_clock_never_goes_backwards() {
    let first = TickCount::now();
    let second = TickCount::now();
    assert!(second >= first);
    assert!(second.get() >= first.get());
}

#[test]
fn spans_and_differences_behave_like_the_cpp() {
    let start = TickCount::now();
    let later = start + TickCountDiff::new(50);
    assert_eq!((later - start).get(), 50);
    assert_eq!((start - later).get(), -50);

    let mut diff = TickCountDiff::new(10);
    diff += 5;
    diff -= 3;
    diff *= 2;
    assert_eq!(diff.get(), 24);
    assert_eq!(i64::from(diff), 24);

    let mut tick = TickCount::now();
    let before = tick;
    tick += TickCountDiff::new(100);
    assert_eq!((tick - before).get(), 100);
    tick -= TickCountDiff::new(100);
    assert_eq!(tick, before);
}

#[test]
fn the_first_reading_is_valid() {
    // `0` means "invalid", and a real reading starts at `sg_tick_init`, so
    // the two can never be confused.
    assert!(TickCount::now().is_valid());
    assert!(marsrs_comm::tickcount::gettickcount() > 0);
    let mut tick = TickCount::invalid();
    tick.refresh();
    assert!(tick.is_valid(), "refreshing makes it valid again");
}

#[test]
fn a_tick_count_nothing_wrote_into_reads_as_long_expired() {
    // Every reading the C++ takes has `sg_tick_init` — two billion
    // milliseconds, about twenty-three days — under it, so a `0` is that far
    // in the past. An origin of zero would make a never-set reading look
    // fresh, and every `gettickspan() > timeout` in the tree would call a
    // timeout that should have gone off still pending.
    let never = TickCount::invalid();
    assert!(
        never.tickspan().get() >= 2_000_000_000,
        "a zero tick count is about twenty-three days old"
    );
    assert!(never.tickspan().get() > 60_000, "older than any timeout");
    assert!(TickCount::now().get() >= 2_000_000_000);
}

#[test]
fn a_zero_tick_count_is_invalid() {
    assert!(!TickCount::invalid().is_valid());
    assert!(TickCount::invalid().get() == 0);
    let mut tick = TickCount::now();
    tick.set_invalid();
    assert!(!tick.is_valid());
    // refreshing makes it valid again, with the reading of the moment it
    // refreshed at — so what it holds is a count the clock has since
    // passed, and not one it has not reached yet
    tick.refresh();
    let refreshed = tick.get();
    assert!(tick.is_valid(), "a refresh left the count invalid");
    assert!(refreshed > 0, "a refresh left the count at zero");
    assert!(
        TickCount::now().get() >= refreshed,
        "the count is ahead of the clock: {refreshed} against {}",
        TickCount::now().get()
    );
}

#[test]
fn tickspan_is_not_negative() {
    let start = TickCount::now();
    assert!(start.tickspan().get() >= 0);
}

#[test]
fn the_singleton_hands_out_the_same_instance() {
    use marsrs_comm::singleton::Singleton;

    static INSTANCE: Singleton<String> = Singleton::new();
    assert!(INSTANCE.get().is_none());
    assert_eq!(INSTANCE.get_or_init(|| "one".to_owned()), "one");
    assert_eq!(INSTANCE.get_or_init(|| "two".to_owned()), "one");
    assert_eq!(INSTANCE.instance(), "one");

    static OTHER: Singleton<u32> = Singleton::new();
    assert!(OTHER.set(1).is_ok());
    assert_eq!(OTHER.set(2), Err(2));
    assert_eq!(OTHER.get(), Some(&1));
}
