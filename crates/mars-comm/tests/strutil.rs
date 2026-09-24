//! Behaviour of `mars-comm::strutil`, checked against what the C++ in
//! mars/comm/strutil.cc does for the same input.

use mars_comm::strutil::{self, Tokenizer, DEFAULT_DELIMITERS};

#[test]
fn url_encode_keeps_unreserved_and_escapes_the_rest() {
    assert_eq!(strutil::url_encode("aZ09.-_*"), "aZ09.-_*");
    assert_eq!(strutil::url_encode("a b"), "a+b");
    assert_eq!(strutil::url_encode("a/b?c=d"), "a%2Fb%3Fc%3Dd");
    // upper-case hex digits, as the "%02X" of the C++
    assert_eq!(strutil::url_encode("\n"), "%0A");
    assert_eq!(strutil::url_encode("中"), "%E4%B8%AD");

    let mut out = "x:".to_owned();
    strutil::url_encode_into("a b", &mut out);
    assert_eq!(out, "x:a+b");
}

#[test]
fn trims_ascii_whitespace_only() {
    assert_eq!(strutil::trim_left("  \t\na"), "a");
    assert_eq!(strutil::trim_right("a \r\n"), "a");
    assert_eq!(strutil::trim(" \t\n\x0b\x0c\r a b \r"), "a b");
    assert_eq!(strutil::trim("   "), "");
    assert_eq!(strutil::trim(""), "");
}

#[test]
fn case_conversion_is_ascii_only() {
    assert_eq!(strutil::cast_lower("AbC1Ä"), "abc1Ä");
    assert_eq!(strutil::cast_upper("aBc1ö"), "ABC1ö");
    let mut s = "MiXeD".to_owned();
    strutil::to_lower_in_place(&mut s);
    assert_eq!(s, "mixed");
    strutil::to_upper_in_place(&mut s);
    assert_eq!(s, "MIXED");
}

#[test]
fn prefix_and_suffix() {
    assert!(strutil::starts_with("mars-stn", "mars"));
    assert!(!strutil::starts_with("mars-stn", "stn"));
    assert!(strutil::ends_with("mars-stn", "stn"));
    assert!(!strutil::ends_with("mars-stn", "mars"));
}

#[test]
fn splits_on_a_delimiter_set() {
    // only the given characters separate, so " c" keeps its leading space
    assert_eq!(strutil::split_token("a,b; c", ",;"), vec!["a", "b", " c"]);
    assert_eq!(strutil::split_token("a,b; c", ",; "), vec!["a", "b", "c"]);
    assert_eq!(strutil::split_token("", ","), Vec::<&str>::new());
    assert_eq!(strutil::split_token("a", "a"), Vec::<&str>::new());
    assert_eq!(DEFAULT_DELIMITERS, " \t\n\r;:,.?");
    assert_eq!(
        Tokenizer::with_default_delimiters("a b,c").collect::<Vec<_>>(),
        vec!["a", "b", "c"]
    );
    // the iterator state advances, exactly like Tokenizer::NextToken
    let mut tokenizer = Tokenizer::new("a b c", " ");
    assert_eq!(tokenizer.next(), Some("a"));
    assert_eq!(tokenizer.next(), Some("b"));
    assert_eq!(tokenizer.next(), Some("c"));
    assert_eq!(tokenizer.next(), None);
}

#[test]
fn merges_tokens_with_a_delimiter() {
    assert_eq!(
        strutil::merge_token(&["a", "b"], ","),
        Some("a,b".to_owned())
    );
    assert_eq!(strutil::merge_token(&["a"], ","), Some("a".to_owned()));
    assert_eq!(strutil::merge_token(&[], ","), None);
    assert_eq!(strutil::merge_token(&["a"], ""), None);
}

#[test]
fn hex_round_trips() {
    assert_eq!(strutil::hex2str(&[0x00, 0x0f, 0xff]), "000fff");
    assert_eq!(strutil::hex2str("AB".as_bytes()), "4142");
    assert_eq!(strutil::str2hex("000fff"), Some(vec![0x00, 0x0f, 0xff]));
    assert_eq!(strutil::str2hex("4142"), Some(b"AB".to_vec()));
    // odd length: the trailing nibble is dropped, as in the C++
    assert_eq!(strutil::str2hex("41a"), Some(vec![0x41]));
    // above 1024 input characters the C++ refuses
    assert_eq!(strutil::str2hex(&"ab".repeat(600)), None);
    assert_eq!(strutil::str2hex("zz"), None);
}

#[test]
fn replaces_every_occurrence() {
    assert_eq!(strutil::replace_char("a@b@c", '@', '.'), "a.b.c");
    assert_eq!(strutil::replace_char("a@b", 'x', '.'), "a@b");
}

#[test]
fn takes_the_file_name_of_a_path() {
    assert_eq!(
        strutil::file_name_from_path("/var/log/mars.xlog"),
        "mars.xlog"
    );
    assert_eq!(
        strutil::file_name_from_path("C:\\log\\mars.xlog"),
        "mars.xlog"
    );
    // a trailing separator has no name after it: the C++ returns the input
    assert_eq!(strutil::file_name_from_path("/var/log/"), "/var/log/");
    assert_eq!(strutil::file_name_from_path("mars.xlog"), "mars.xlog");
}

#[test]
fn finds_a_substring_ignoring_case() {
    assert_eq!(strutil::ci_find_substr("Hello World", "world", 0), Some(6));
    assert_eq!(strutil::ci_find_substr("Hello World", "hello", 0), Some(0));
    assert_eq!(strutil::ci_find_substr("Hello World", "world", 7), None);
    assert_eq!(strutil::ci_find_substr("abc", "", 1), Some(1));
    assert_eq!(strutil::ci_find_substr("abc", "d", 0), None);
}

#[test]
fn a_needle_longer_than_the_haystack_is_not_found() {
    // `pos == 0` used to leave a one-element range whose slice ran past the
    // end of the haystack, so this panicked instead of answering `None`.
    assert_eq!(strutil::ci_find_substr("a", "long", 0), None);
    assert_eq!(strutil::ci_find_substr("", "long", 0), None);
    assert_eq!(strutil::ci_find_substr("abc", "abcd", 1), None);
}

#[test]
fn tokens_are_split_on_character_boundaries() {
    // The delimiter is a multi-byte character: matching it byte by byte would
    // cut "Ã" in half and panic on the slice.
    assert_eq!(
        Tokenizer::new("\u{00c3}x", "\u{00e9}").collect::<Vec<_>>(),
        vec!["\u{00c3}x"]
    );
    assert_eq!(
        Tokenizer::new("a\u{00e9}b", "\u{00e9}").collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert_eq!(
        Tokenizer::new("\u{4e2d}\u{6587} a", " ").collect::<Vec<_>>(),
        vec!["\u{4e2d}\u{6587}", "a"]
    );
}

#[test]
fn md5_matches_the_reference_vectors() {
    assert_eq!(strutil::buffer_md5(b""), "d41d8cd98f00b204e9800998ecf8427e");
    assert_eq!(
        strutil::buffer_md5(b"abc"),
        "900150983cd24fb0d6963f7d28e17f72"
    );
    assert_eq!(strutil::digest_to_base16(&[0x00, 0xff]), "00ff");
    assert_eq!(
        strutil::md5_digest_to_base16(&[0u8; 16]),
        "00000000000000000000000000000000"
    );
}

#[test]
fn c_strings_are_read_safely() {
    use std::ffi::CString;
    use std::os::raw::c_char;

    let a = CString::new("12").unwrap();
    let b = CString::new("12").unwrap();
    let c = CString::new("13").unwrap();
    let empty = CString::new("").unwrap();
    let null: *const c_char = std::ptr::null();

    unsafe {
        assert_eq!(
            strutil::cstr_to_string_safe(a.as_ptr()),
            Some("12".to_owned())
        );
        assert_eq!(strutil::cstr_to_string_safe(null), None);
        assert!(!strutil::cstr_null_or_empty(a.as_ptr()));
        assert!(strutil::cstr_null_or_empty(empty.as_ptr()));
        assert!(strutil::cstr_null_or_empty(null));
        assert!(strutil::cstr_cmp_safe(a.as_ptr(), b.as_ptr()));
        assert!(!strutil::cstr_cmp_safe(a.as_ptr(), c.as_ptr()));
        assert!(!strutil::cstr_cmp_safe(a.as_ptr(), null));
        assert_eq!(strutil::cstr_to_i32_safe(a.as_ptr(), -1), 12);
        assert_eq!(strutil::cstr_to_i32_safe(null, -1), -1);
        assert_eq!(strutil::cstr_to_i32_safe(c.as_ptr(), 0), 13);
        // `atoi` reads the numeric prefix and nothing else
        let prefixed = CString::new("12ms").unwrap();
        let negative = CString::new("  -3abc").unwrap();
        let not_a_number = CString::new("abc").unwrap();
        let huge = CString::new("99999999999999").unwrap();
        assert_eq!(strutil::cstr_to_i32_safe(prefixed.as_ptr(), -1), 12);
        assert_eq!(strutil::cstr_to_i32_safe(negative.as_ptr(), -1), -3);
        assert_eq!(strutil::cstr_to_i32_safe(not_a_number.as_ptr(), -1), 0);
        assert_eq!(
            strutil::cstr_to_i32_safe(huge.as_ptr(), -1),
            i32::MAX,
            "atoi saturates"
        );
    }
}

#[test]
fn formats_hex_octal_and_joins() {
    assert_eq!(strutil::to_hex_string(255u32), "ff");
    assert_eq!(strutil::to_oct_string(8u32), "10");
    // the C++ appends the separator after every item, so the postfix closes it
    assert_eq!(strutil::join_to_string(["a", "b"], ",", "{", "}"), "{a,b,}");
    assert_eq!(strutil::join_to_string_for_log(["a", "b"]), "{a,b,}");
    assert_eq!(strutil::join_to_string::<_, &str>([], ",", "{", "}"), "");
}
