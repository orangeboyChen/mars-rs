//! `mars/comm/shuffle.h` — what the one shuffle mars asks for does with a
//! draw that is not below the bound it was asked for.
//!
//! Two tests for it, because the two builds answer it differently: a debug
//! build says an rng that broke its contract did, where it happens, and
//! every build clamps the draw instead of swapping past the end of the
//! slice.

use marsrs_comm::shuffle::random_shuffle;

#[cfg(debug_assertions)]
#[test]
#[should_panic]
fn a_draw_outside_the_bound_is_said_out_loud() {
    let mut items = [0, 1, 2, 3];
    random_shuffle(&mut items, &mut |_bound| usize::MAX);
}

#[cfg(not(debug_assertions))]
#[test]
fn a_draw_outside_the_bound_is_clamped_to_the_position_being_walked() {
    let mut items = [0, 1, 2, 3];
    random_shuffle(&mut items, &mut |_bound| usize::MAX);
    // Clamped to the position itself, which is the swap that changes
    // nothing: every draw is the position it is clamped to, so the slice is
    // left in the order it was handed over as. Sorted first, as this used to
    // be, every permutation of the four answers it — including every one a
    // shuffle that swapped something real leaves behind.
    assert_eq!(items, [0, 1, 2, 3]);
}
