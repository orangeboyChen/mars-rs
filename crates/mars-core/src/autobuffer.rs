//! Port of `mars/comm/autobuffer.h` / `autobuffer.cc`.
//!
//! `AutoBuffer` is an owned, self-growing byte buffer with the same
//! pos / length / capacity model as `PtrBuffer`.

use crate::ptrbuffer::Seek;

/// An owned growable byte buffer.
#[derive(Debug, Clone, Default)]
pub struct AutoBuffer {
    data: Vec<u8>,
    pos: usize,
    length: usize,
}

impl AutoBuffer {
    pub fn new() -> Self {
        Self::with_capacity(128)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity.max(1)),
            pos: 0,
            length: 0,
        }
    }

    /// Builds a buffer that already holds `_data`.
    pub fn from_bytes(data: Vec<u8>) -> Self {
        let length = data.len();
        Self {
            data,
            pos: 0,
            length,
        }
    }

    fn fit(&mut self, needed: usize) {
        if self.data.len() < needed {
            self.data.resize(needed, 0);
        }
    }

    /// Reserves room for `pos + ready_to_write` bytes and raises the logical
    /// length accordingly. Mirrors `AllocWrite(ready_to_write, true)`.
    pub fn alloc_write(&mut self, ready_to_write: usize) {
        self.fit(self.pos + ready_to_write);
        self.length = self.length.max(self.pos + ready_to_write);
    }

    /// `AddCapacity`.
    pub fn add_capacity(&mut self, extra: usize) {
        self.data.reserve(extra);
    }

    /// Appends `_src` at the cursor and advances it.
    pub fn write(&mut self, src: &[u8]) {
        self.write_at(self.pos, src);
        self.seek(src.len() as isize, Seek::Cur);
    }

    /// Writes `_src` at `_pos`, growing the buffer when needed.
    pub fn write_at(&mut self, pos: usize, src: &[u8]) {
        debug_assert!(pos <= self.length);
        self.fit(pos + src.len());
        self.data[pos..pos + src.len()].copy_from_slice(src);
        self.length = self.length.max(pos + src.len());
    }

    /// Reads `_dst.len()` bytes from `_pos`.
    pub fn read_at(&self, pos: usize, dst: &mut [u8]) -> usize {
        if pos >= self.length {
            return 0;
        }
        let n = (self.length - pos).min(dst.len());
        dst[..n].copy_from_slice(&self.data[pos..pos + n]);
        n
    }

    pub fn seek(&mut self, offset: isize, origin: Seek) {
        let target = match origin {
            Seek::Start => offset,
            Seek::Cur => self.pos as isize + offset,
            Seek::End => self.length as isize + offset,
        };
        self.pos = target.clamp(0, self.length as isize) as usize;
    }

    pub fn set_length(&mut self, pos: usize, length: usize) {
        debug_assert!(pos <= length);
        self.fit(length);
        self.length = length;
        self.seek(pos as isize, Seek::Start);
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data[..self.length]
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data[..self.length]
    }

    /// Bytes `[pos, length)`.
    pub fn pos_slice(&self) -> &[u8] {
        &self.data[self.pos..self.length]
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn pos_length(&self) -> usize {
        self.length - self.pos
    }

    pub fn len(&self) -> usize {
        self.length
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn capacity(&self) -> usize {
        self.data.capacity()
    }

    /// Truncates the logical length to zero but keeps the allocation.
    pub fn reset(&mut self) {
        self.pos = 0;
        self.length = 0;
    }

    /// Consumes the buffer, returning the bytes in `[0, length)`.
    pub fn into_vec(mut self) -> Vec<u8> {
        self.data.truncate(self.length);
        self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_write_sets_length() {
        let mut buf = AutoBuffer::new();
        buf.alloc_write(10);
        assert_eq!(buf.len(), 10);
        assert_eq!(buf.as_slice(), &[0u8; 10]);
    }

    #[test]
    fn write_grows_buffer() {
        let mut buf = AutoBuffer::with_capacity(2);
        buf.write(b"hello");
        assert_eq!(buf.len(), 5);
        assert_eq!(buf.pos(), 5);
        assert_eq!(buf.as_slice(), b"hello");
        buf.write(b" world");
        assert_eq!(buf.as_slice(), b"hello world");
    }

    #[test]
    fn write_at_keeps_length_when_inside() {
        let mut buf = AutoBuffer::new();
        buf.write(b"abcd");
        buf.write_at(1, b"z");
        assert_eq!(buf.as_slice(), b"azcd");
        assert_eq!(buf.len(), 4);
    }

    #[test]
    fn set_length_and_pos() {
        let mut buf = AutoBuffer::new();
        buf.write(b"abcdef");
        buf.set_length(2, 6);
        assert_eq!(buf.pos(), 2);
        assert_eq!(buf.pos_slice(), b"cdef");
    }

    #[test]
    fn read_at_respects_length() {
        let mut buf = AutoBuffer::new();
        buf.write(b"abc");
        let mut out = [0u8; 8];
        assert_eq!(buf.read_at(1, &mut out), 2);
        assert_eq!(&out[..2], b"bc");
    }

    #[test]
    fn into_vec_truncates_to_length() {
        let mut buf = AutoBuffer::new();
        buf.alloc_write(4);
        buf.write_at(0, b"ab");
        buf.set_length(0, 3);
        assert_eq!(buf.into_vec(), b"ab\0".to_vec());
    }
}
