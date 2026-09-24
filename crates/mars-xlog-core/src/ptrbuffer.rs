//! Port of `mars/comm/ptrbuffer.h` / `ptrbuffer.cc`.
//!
//! A `PtrBuffer` borrows a fixed-size region of memory (typically the mmap'd
//! xlog cache file) and keeps a *pos* / *length* / *max_length* triple on top
//! of it. Unlike the C++ original it never owns the memory and never performs
//! pointer arithmetic on raw pointers, so it is safe Rust.

/// `PtrBuffer::Seek` origin, mirrors `PtrBuffer::TSeek`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seek {
    Start,
    Cur,
    End,
}

/// A cursor over a caller-owned byte region.
#[derive(Debug)]
pub struct PtrBuffer<'a> {
    data: &'a mut [u8],
    pos: usize,
    length: usize,
}

impl<'a> PtrBuffer<'a> {
    /// Attach to `_data` with an initial logical length of `_len`.
    ///
    /// # Panics
    /// Panics when `_len` exceeds `_data.len()`.
    pub fn attach(data: &'a mut [u8], len: usize) -> Self {
        assert!(
            len <= data.len(),
            "length {len} exceeds max_length {}",
            data.len()
        );
        Self {
            data,
            pos: 0,
            length: len,
        }
    }

    /// Attach to `_data` with a zero logical length.
    pub fn new(data: &'a mut [u8]) -> Self {
        Self {
            data,
            pos: 0,
            length: 0,
        }
    }

    /// Re-attach to a different region, resetting the cursor.
    pub fn reset_attach(&mut self, data: &'a mut [u8], len: usize) {
        assert!(
            len <= data.len(),
            "length {len} exceeds max_length {}",
            data.len()
        );
        self.data = data;
        self.pos = 0;
        self.length = len;
    }

    /// Detach: the buffer behaves as if it wrapped an empty region.
    pub fn reset(&mut self) {
        self.data = &mut [];
        self.pos = 0;
        self.length = 0;
    }

    /// Whole backing region (`Ptr()` in the C++ code).
    pub fn as_slice(&self) -> &[u8] {
        self.data
    }

    /// Mutable whole backing region.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.data
    }

    /// Bytes `[pos, max_length)` — where new data is written.
    pub fn pos_slice_mut(&mut self) -> &mut [u8] {
        let pos = self.pos;
        &mut self.data[pos..]
    }

    /// Logical length.
    pub fn len(&self) -> usize {
        self.length
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Capacity of the backing region (`MaxLength()` in the C++ code).
    pub fn max_length(&self) -> usize {
        self.data.len()
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    /// `length - pos`
    pub fn pos_length(&self) -> usize {
        self.length - self.pos
    }

    /// Writes `_src` at the current position and advances it.
    ///
    /// Returns the number of bytes actually written; the write is truncated so
    /// that it never runs past the backing region.
    pub fn write(&mut self, src: &[u8]) -> usize {
        let written = self.write_at(self.pos, src);
        self.seek(written as isize, Seek::Cur);
        written
    }

    /// Writes `_src` at `_pos` without moving the cursor.
    pub fn write_at(&mut self, pos: usize, src: &[u8]) -> usize {
        debug_assert!(pos <= self.length);
        let copy_len = src.len().min(self.data.len().saturating_sub(pos));
        self.data[pos..pos + copy_len].copy_from_slice(&src[..copy_len]);
        self.length = self.length.max(copy_len + pos);
        copy_len
    }

    /// Reads into `_dst` starting at `_pos`; returns the number of bytes read.
    pub fn read_at(&self, pos: usize, dst: &mut [u8]) -> usize {
        if pos >= self.length {
            return 0;
        }
        let n = (self.length - pos).min(dst.len());
        dst[..n].copy_from_slice(&self.data[pos..pos + n]);
        n
    }

    /// Reads at the cursor and advances it.
    pub fn read(&mut self, dst: &mut [u8]) -> usize {
        let n = self.read_at(self.pos, dst);
        self.seek(n as isize, Seek::Cur);
        n
    }

    /// Moves the cursor, clamped to `[0, length]`.
    pub fn seek(&mut self, offset: isize, origin: Seek) {
        let target = match origin {
            Seek::Start => offset,
            Seek::Cur => self.pos as isize + offset,
            Seek::End => self.length as isize + offset,
        };
        self.pos = target.clamp(0, self.length as isize) as usize;
    }

    /// Sets the logical length (clamped to the backing region) and moves the
    /// cursor to `_pos` (clamped to the new length).
    pub fn set_length(&mut self, pos: usize, length: usize) {
        debug_assert!(pos <= length);
        self.length = length.min(self.data.len());
        self.seek(pos as isize, Seek::Start);
    }

    /// Zeroes the whole backing region and resets the cursor.
    pub fn clear(&mut self) {
        self.data.fill(0);
        self.pos = 0;
        self.length = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_and_length() {
        let mut backing = vec![0u8; 16];
        let mut buf = PtrBuffer::new(&mut backing);
        assert_eq!(buf.max_length(), 16);
        assert_eq!(buf.write(b"hello"), 5);
        assert_eq!(buf.len(), 5);
        assert_eq!(buf.pos(), 5);
        assert_eq!(&buf.as_slice()[..5], b"hello");
    }

    #[test]
    fn write_is_truncated_at_max_length() {
        let mut backing = vec![0u8; 4];
        let mut buf = PtrBuffer::new(&mut backing);
        assert_eq!(buf.write(b"hello world"), 4);
        assert_eq!(buf.len(), 4);
        assert_eq!(buf.as_slice(), b"hell");
    }

    #[test]
    fn write_at_does_not_move_pos() {
        let mut backing = vec![0u8; 16];
        let mut buf = PtrBuffer::new(&mut backing);
        buf.write(b"abcd");
        buf.write_at(0, b"xy");
        assert_eq!(buf.as_slice()[..4], *b"xycd");
        assert_eq!(buf.pos(), 4);
    }

    #[test]
    fn set_length_clamps_and_seeks() {
        let mut backing = vec![0u8; 8];
        let mut buf = PtrBuffer::new(&mut backing);
        buf.set_length(4, 64);
        assert_eq!(buf.len(), 8);
        assert_eq!(buf.pos(), 4);
        buf.set_length(6, 6);
        assert_eq!(buf.len(), 6);
        assert_eq!(buf.pos(), 6);
    }

    #[test]
    fn seek_is_clamped() {
        let mut backing = vec![0u8; 8];
        let mut buf = PtrBuffer::new(&mut backing);
        buf.write(b"abcd");
        buf.seek(-100, Seek::Cur);
        assert_eq!(buf.pos(), 0);
        buf.seek(100, Seek::Cur);
        assert_eq!(buf.pos(), 4);
        buf.seek(0, Seek::End);
        assert_eq!(buf.pos(), 4);
    }

    #[test]
    fn read_round_trip() {
        let mut backing = vec![0u8; 8];
        let mut buf = PtrBuffer::new(&mut backing);
        buf.write(b"abcdef");
        buf.set_length(0, 6);
        let mut out = [0u8; 3];
        assert_eq!(buf.read(&mut out), 3);
        assert_eq!(&out, b"abc");
        assert_eq!(buf.pos(), 3);
        assert_eq!(buf.pos_length(), 3);
    }

    #[test]
    fn clear_zeroes_backing_region() {
        let mut backing = vec![0u8; 4];
        {
            let mut buf = PtrBuffer::new(&mut backing);
            buf.write(b"abcd");
            buf.clear();
        }
        assert_eq!(backing, vec![0u8; 4]);
    }
}
