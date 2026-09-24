//! `mars-xlog-core` — foundation primitives for the Rust port of Mars xlog.
//!
//! This crate is a faithful, dependency-free port of the byte-buffer types in
//! `mars/comm` that the xlog pipeline is built on:
//!
//! | C++                        | Rust                              |
//! |----------------------------|-----------------------------------|
//! | `mars::comm::PtrBuffer`    | [`ptrbuffer::PtrBuffer`]          |
//! | `mars::comm::AutoBuffer`   | [`autobuffer::AutoBuffer`]        |
//!
//! Keeping these in a leaf crate means the compression, crypto and appender
//! layers can be ported (and reviewed) independently.

#![deny(unsafe_code)]

pub mod autobuffer;
pub mod ptrbuffer;

pub use autobuffer::AutoBuffer;
pub use ptrbuffer::{PtrBuffer, Seek};

/// Little-endian helpers used by the xlog on-disk format.
///
/// The C++ implementation `memcpy`s `uint16_t` / `uint32_t` straight into the
/// log header, so the wire format is native-endian. Every platform Mars ships
/// on is little-endian; these helpers pin that down explicitly instead of
/// relying on it.
pub mod le {
    /// Reads a little-endian `u16`.
    pub fn read_u16(buf: &[u8], offset: usize) -> u16 {
        let b: [u8; 2] = buf[offset..offset + 2]
            .try_into()
            .expect("u16 needs 2 bytes");
        u16::from_le_bytes(b)
    }

    /// Writes a little-endian `u16`.
    pub fn write_u16(buf: &mut [u8], offset: usize, value: u16) {
        buf[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    /// Reads a little-endian `u32`.
    pub fn read_u32(buf: &[u8], offset: usize) -> u32 {
        let b: [u8; 4] = buf[offset..offset + 4]
            .try_into()
            .expect("u32 needs 4 bytes");
        u32::from_le_bytes(b)
    }

    /// Writes a little-endian `u32`.
    pub fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
        buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
}

/// Returns the current local hour `0..=23`, used for the xlog header's
/// begin/end hour fields.
pub fn local_hour() -> u8 {
    use chrono::Timelike;

    chrono::Local::now().hour() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use le::*;

    #[test]
    fn le_round_trip() {
        let mut buf = [0u8; 8];
        write_u16(&mut buf, 0, 0x1234);
        write_u32(&mut buf, 2, 0xdead_beef);
        assert_eq!(read_u16(&buf, 0), 0x1234);
        assert_eq!(read_u32(&buf, 2), 0xdead_beef);
    }

    #[test]
    fn local_hour_is_valid() {
        assert!(local_hour() < 24);
    }
}
