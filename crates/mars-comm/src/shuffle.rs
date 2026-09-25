//! `mars/comm/shuffle.h` — `mars::comm::random_shuffle`, the one shuffle the
//! whole of mars asks for.
//!
//! Three places in the C++ reach for it: `net_source.cc` shuffles the backup
//! pairs it appended to the list, `simple_ipport_sort.cc` opens its sort with
//! it, and `sdt/src/checkimpl/http_detector.cc` shuffles what dns answered
//! before it picks an address out of it. The last of the three is a host
//! decision in this port — the port's sdt takes the addresses it probes from a
//! seam, so putting them in an order is whoever answers that seam — and the
//! other two are `mars-stn`, which calls this function.
//!
//! The C++ draws its randomness from OpenSSL's `RAND_bytes`, once per
//! position. The port takes the draw as a callback instead, the way the rest of
//! it takes its platform: `random(bound)` has to answer with a number below
//! `bound`, which is the contract every rng in this repository already keeps.
//! The callback is what makes the order a test can pin down.

/// `random_shuffle(first, last)` — Fisher-Yates, walking the slice backwards
/// and swapping every position with one drawn from the whole slice up to and
/// including it.
///
/// A slice of nothing or of one thing is left alone, and its rng is never
/// asked for anything.
pub fn random_shuffle<T>(items: &mut [T], random: &mut dyn FnMut(usize) -> usize) {
    for index in (1..items.len()).rev() {
        let picked = random(index + 1);
        items.swap(index, picked);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An rng of the port's own: splitmix64, the finalizer `mars-stn`'s
    /// xorshift is a weaker cousin of, so nothing here depends on what the
    /// platform's `rand()` happens to say.
    fn rng(seed: u64) -> impl FnMut(usize) -> usize {
        let mut state = seed;
        move |bound: usize| {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            ((z ^ (z >> 31)) % bound as u64) as usize
        }
    }

    #[test]
    fn a_slice_that_cannot_be_reordered_is_left_alone() {
        let draws = std::cell::Cell::new(0usize);
        let mut count = |_bound: usize| {
            draws.set(draws.get() + 1);
            0
        };

        let mut empty: [u8; 0] = [];
        random_shuffle(&mut empty, &mut count);
        assert_eq!(empty, []);

        let mut one = [7u8];
        random_shuffle(&mut one, &mut count);
        assert_eq!(one, [7]);
        // `for (i = n - 1; i > 0; --i)`: nothing to walk, nothing to draw
        assert_eq!(draws.get(), 0);

        // ... and the first position that is walked is the second one
        let mut two = [0u8, 1];
        random_shuffle(&mut two, &mut count);
        assert_eq!(draws.get(), 1);
        assert_eq!(two, [1, 0]);
    }

    /// `for (i = n - 1; i > 0; --i)` and `seed % (i + 1)`: one draw for every
    /// position but the first, and the bound is the position plus one.
    #[test]
    fn every_position_after_the_first_draws_one_number_below_it() {
        let mut items = [0, 1, 2, 3, 4];
        let mut bounds = Vec::new();
        random_shuffle(&mut items, &mut |bound| {
            bounds.push(bound);
            0
        });
        assert_eq!(bounds, vec![5, 4, 3, 2]);
    }

    /// What the C++'s walk does when every draw is the same: the last position
    /// is swapped with the first, then the next one with whatever landed there.
    #[test]
    fn a_draw_of_zero_every_time_walks_the_last_position_to_the_front() {
        let mut items = [0, 1, 2, 3];
        random_shuffle(&mut items, &mut |_| 0);
        assert_eq!(items, [1, 2, 3, 0]);
    }

    #[test]
    fn a_shuffle_of_two_is_the_draw_it_made() {
        let mut items = [0, 1];
        random_shuffle(&mut items, &mut |_| 0);
        assert_eq!(items, [1, 0]);

        // a draw of the position itself is the swap that changes nothing
        let mut items = [0, 1];
        random_shuffle(&mut items, &mut |_| 1);
        assert_eq!(items, [0, 1]);
    }

    #[test]
    fn the_shuffle_only_moves_what_it_is_given() {
        let mut items = (0..64).collect::<Vec<_>>();
        let before = items.clone();
        let mut shuffle = rng(0x5EED);
        random_shuffle(&mut items, &mut shuffle);

        // it is a permutation and not a sample: nothing is lost and nothing is
        // added, whatever the draws were
        let mut sorted = items.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, before);
        // ... and it did move them
        assert_ne!(items, before);
    }

    /// The shuffles are not all the same shuffle: what the port's own rng draws
    /// spreads a slice of six over every order it can be in.
    #[test]
    fn the_orders_a_slice_of_six_can_be_put_in_all_come_up() {
        let mut seen = std::collections::BTreeSet::new();
        let mut shuffle = rng(0xC0FFEE);
        for _ in 0..20_000 {
            let mut items = [0, 1, 2, 3, 4, 5];
            random_shuffle(&mut items, &mut shuffle);
            seen.insert(items);
        }
        assert_eq!(seen.len(), 720, "{} of the 720 orders", seen.len());
    }

    /// `T` is unconstrained: the C++ swaps iterators, and the port swaps
    /// whatever the caller has a slice of.
    #[test]
    fn anything_can_be_shuffled() {
        let mut items = ["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let mut shuffle = rng(7);
        random_shuffle(&mut items, &mut shuffle);
        let mut sorted = items.clone();
        sorted.sort();
        assert_eq!(sorted, ["a".to_owned(), "b".to_owned(), "c".to_owned()]);
    }
}
