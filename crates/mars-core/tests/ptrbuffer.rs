//! The uncovered corners of `PtrBuffer`: attaching, re-attaching, detaching,
//! the three seek origins, clamping and the truncated read/write paths.

use mars_core::ptrbuffer::{PtrBuffer, Seek};

#[test]
fn attach_sets_the_logical_length_and_new_starts_empty() {
    let mut region = [0u8; 8];
    let buffer = PtrBuffer::attach(&mut region, 4);
    assert_eq!(buffer.len(), 4);
    assert_eq!(buffer.max_length(), 8);
    assert_eq!(buffer.pos(), 0);
    assert_eq!(buffer.pos_length(), 4);
    assert!(!buffer.is_empty());

    let mut region = [1u8, 2, 3];
    let buffer = PtrBuffer::new(&mut region);
    assert_eq!(buffer.len(), 0);
    assert!(buffer.is_empty());
    assert_eq!(buffer.pos_length(), 0);
}

#[test]
#[should_panic]
fn attaching_past_the_region_panics() {
    let mut region = [0u8; 4];
    PtrBuffer::attach(&mut region, 5);
}

#[test]
fn reattach_swaps_the_region_and_resets_the_cursor() {
    let mut first = [0u8; 4];
    let mut buffer = PtrBuffer::new(&mut first);
    buffer.write(b"abcd");
    assert_eq!(buffer.pos(), 4);

    let mut second = [0u8; 2];
    buffer.reset_attach(&mut second, 0);
    assert_eq!(buffer.pos(), 0);
    assert_eq!(buffer.max_length(), 2);
    assert_eq!(buffer.as_slice(), &[0u8; 2]);
}

#[test]
#[should_panic]
fn reattaching_past_the_region_panics() {
    let mut first = [0u8; 4];
    let mut buffer = PtrBuffer::new(&mut first);
    let mut second = [0u8; 2];
    buffer.reset_attach(&mut second, 3);
}

#[test]
fn reset_detaches_the_buffer() {
    let mut region = [7u8; 4];
    let mut buffer = PtrBuffer::attach(&mut region, 4);
    buffer.reset();
    assert_eq!(buffer.len(), 0);
    assert_eq!(buffer.max_length(), 0);
    assert!(buffer.is_empty());
    assert_eq!(buffer.as_slice(), &[]);
    assert_eq!(buffer.pos_slice_mut(), &[]);
}

#[test]
fn seek_clamps_to_the_logical_length_and_the_three_origins_move_the_cursor() {
    let mut region = [0u8; 8];
    let mut buffer = PtrBuffer::attach(&mut region, 4);
    buffer.seek(2, Seek::Start);
    assert_eq!(buffer.pos(), 2);
    buffer.seek(1, Seek::Cur);
    assert_eq!(buffer.pos(), 3);
    buffer.seek(-1, Seek::End);
    assert_eq!(buffer.pos(), 3);
    buffer.seek(100, Seek::Start);
    assert_eq!(buffer.pos(), 4, "clamped to length");
    buffer.seek(-100, Seek::Cur);
    assert_eq!(buffer.pos(), 0, "clamped to zero");
}

#[test]
fn writes_are_truncated_and_reads_stop_at_the_logical_length() {
    let mut region = [0u8; 4];
    let mut buffer = PtrBuffer::new(&mut region);
    assert_eq!(buffer.write(b"abcdefgh"), 4);
    assert_eq!(buffer.len(), 4);
    assert_eq!(buffer.as_slice(), b"abcd");
    assert_eq!(buffer.write(b"z"), 0);

    let mut out = [0u8; 4];
    buffer.seek(0, Seek::Start);
    assert_eq!(buffer.read(&mut out), 4);
    assert_eq!(&out, b"abcd");
    assert_eq!(buffer.read(&mut out), 0);
    assert_eq!(buffer.read_at(4, &mut out), 0);
    assert_eq!(buffer.read_at(0, &mut out[..2]), 2);
    assert_eq!(&out[..2], b"ab");
}

#[test]
fn write_at_grows_the_length_without_moving_the_cursor() {
    let mut region = [0u8; 8];
    // `write_at` asserts that the position is inside the logical length
    let mut buffer = PtrBuffer::attach(&mut region, 2);
    assert_eq!(buffer.write_at(2, b"xy"), 2);
    assert_eq!(buffer.len(), 4, "the length covers the hole");
    assert_eq!(buffer.pos(), 0, "the cursor did not move");
    assert_eq!(&buffer.as_slice()[..4], &[0, 0, b'x', b'y']);
}

#[test]
fn set_length_clamps_and_moves_the_cursor() {
    let mut region = [0u8; 4];
    let mut buffer = PtrBuffer::new(&mut region);
    buffer.set_length(2, 16);
    assert_eq!(buffer.len(), 4, "clamped to the region");
    assert_eq!(buffer.pos(), 2);
    buffer.set_length(1, 3);
    assert_eq!(buffer.len(), 3);
    assert_eq!(buffer.pos(), 1);
}

#[test]
fn clear_zeroes_the_whole_region() {
    let mut region = [9u8; 4];
    let mut buffer = PtrBuffer::attach(&mut region, 4);
    buffer.clear();
    assert_eq!(buffer.len(), 0);
    assert_eq!(buffer.pos(), 0);
    assert_eq!(buffer.as_slice(), &[0u8; 4]);
}

#[test]
fn as_mut_slice_writes_through_to_the_region() {
    let mut region = [0u8; 4];
    let mut buffer = PtrBuffer::attach(&mut region, 4);
    buffer.as_mut_slice().fill(1);
    assert_eq!(buffer.as_slice(), &[1u8; 4]);
    buffer.seek(2, Seek::Start);
    assert_eq!(buffer.pos_slice_mut(), &[1u8, 1]);
}
