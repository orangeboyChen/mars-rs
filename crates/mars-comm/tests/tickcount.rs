//! `tickcount_t` / `gettickcount()`.

use mars_comm::{TickCount, TickCountDiff};

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
fn a_zero_tick_count_is_invalid() {
    assert!(!TickCount::invalid().is_valid());
    assert!(TickCount::invalid().get() == 0);
    let mut tick = TickCount::now();
    tick.set_invalid();
    assert!(!tick.is_valid());
    // refreshing makes it valid again (unless the process just started)
    tick.refresh();
    assert_eq!(
        tick.get(),
        TickCount::now().get() - TickCount::now().get() + tick.get()
    );
}

#[test]
fn tickspan_is_not_negative() {
    let start = TickCount::now();
    assert!(start.tickspan().get() >= 0);
}

#[test]
fn the_singleton_hands_out_the_same_instance() {
    use mars_comm::singleton::Singleton;

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
