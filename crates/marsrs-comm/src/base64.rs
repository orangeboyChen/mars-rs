//! `mars/comm/crypt/ibase64.h` — base64, which is what a proxy is logged in
//! with: `mars::comm::EncodeBase64` turns `username:password` into the
//! `Basic` of a `Proxy-Authorization`.
//!
//! The C++ writes into a buffer the caller sized with
//! `modp_b64_encode_len(A) == ((A + 2) / 3 * 4 + 1)` — the `+ 1` is the `\0` it
//! terminates with — and answers how many characters it wrote. The port hands
//! back a `String` instead, which is the same characters without the two things
//! a Rust caller would only have to undo.
//!
//! `DecodeBase64` is not ported: nothing in mars calls it.

/// The alphabet `init_conversion_tables` builds: `A`..`Z`, `a`..`z`, `0`..`9`,
/// `+`, `/`.
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `modp_b64_encode_len(A)` without the `\0`: how many characters [`encode`]
/// writes for `len` bytes, which is four for every three of them — the C++'s
/// `(A + 2) / 3 * 4`.
pub fn encoded_len(len: usize) -> usize {
    len.div_ceil(3) * 4
}

/// `EncodeBase64` — `data` as base64, `=`-padded like the C++: a last group of
/// one byte ends in `==` and one of two bytes in `=`.
///
/// Nothing at all is an empty string, which is what the C++ answers `0` for.
pub fn encode(data: &[u8]) -> String {
    let mut encoded = String::with_capacity(encoded_len(data.len()));
    for group in data.chunks(3) {
        let first = group[0] as usize;
        let second = *group.get(1).unwrap_or(&0) as usize;
        let third = *group.get(2).unwrap_or(&0) as usize;

        encoded.push(ALPHABET[first >> 2] as char);
        encoded.push(ALPHABET[((first & 0x03) << 4) | (second >> 4)] as char);
        match group.len() {
            1 => {
                encoded.push('=');
                encoded.push('=');
            }
            2 => {
                encoded.push(ALPHABET[((second & 0x0F) << 2) | (third >> 6)] as char);
                encoded.push('=');
            }
            _ => {
                encoded.push(ALPHABET[((second & 0x0F) << 2) | (third >> 6)] as char);
                encoded.push(ALPHABET[third & 0x3F] as char);
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_nothing() {
        assert_eq!(encode(b""), "");
        assert_eq!(encoded_len(0), 0);
    }

    #[test]
    fn three_bytes_are_four_characters_and_the_rest_is_padded() {
        // the one the C++'s own comment draws out
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn how_many_characters_there_are_is_what_the_c_sizes_its_buffer_for() {
        for len in 0..12 {
            assert_eq!(encode(&vec![b'x'; len]).len(), encoded_len(len));
        }
        // `((A + 2) / 3 * 4 + 1)`, less the `\0`
        assert_eq!(encoded_len(1), 4);
        assert_eq!(encoded_len(3), 4);
        assert_eq!(encoded_len(4), 8);
    }

    #[test]
    fn a_byte_of_every_value_goes_out_the_way_it_came_in() {
        // every byte, in one go: what `+` and `/` are for
        let all: Vec<u8> = (0..=255u8).collect();
        let encoded = encode(&all);
        assert!(encoded.contains('+'));
        assert!(encoded.contains('/'));
        assert_eq!(encoded.len(), encoded_len(256));
        assert_eq!(&encoded[52..56], "Jygp");
    }

    #[test]
    fn the_account_a_proxy_is_logged_in_with() {
        // `username:password`, which is the only thing mars encodes
        assert_eq!(encode(b"u:p"), "dTpw");
        assert_eq!(encode(b"mars:secret"), "bWFyczpzZWNyZXQ=");
    }
}
