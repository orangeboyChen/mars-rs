//! `mars-comm::singleton` — the `OnceLock` equivalent of comm/singleton.h.

use mars_comm::singleton::Singleton;

#[test]
fn the_first_writer_wins() {
    let slot: Singleton<u32> = Singleton::new();
    assert!(slot.get().is_none());
    assert!(slot.set(1).is_ok());
    assert_eq!(slot.set(2), Err(2), "the second Create() is refused");
    assert_eq!(slot.get(), Some(&1));
    assert_eq!(slot.instance(), &1);
    // get_or_init keeps the value that is already there
    assert_eq!(*slot.get_or_init(|| 99), 1);
}

#[test]
#[should_panic]
fn instance_before_create_panics() {
    let slot: Singleton<u32> = Singleton::default();
    slot.instance();
}

#[test]
fn a_singleton_can_be_shared_across_threads() {
    static SLOT: Singleton<String> = Singleton::new();
    let handles: Vec<_> = (0..4)
        .map(|i| {
            std::thread::spawn(move || SLOT.get_or_init(|| format!("made by {i}")))
                .join()
                .unwrap()
        })
        .collect();
    // every thread got the same value: whichever one created it
    let first = handles[0].clone();
    assert!(handles.iter().all(|value| **value == first), "{handles:?}");
}
