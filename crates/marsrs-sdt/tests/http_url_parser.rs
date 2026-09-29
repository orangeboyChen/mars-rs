//! `mars/sdt/src/checkimpl/http_url_parser.h` — one URL, as a host, a port and
//! a path.

use marsrs_sdt::HttpUrlParser;

#[test]
fn a_url_is_a_host_a_port_and_a_path() {
    let url = HttpUrlParser::new("http://www.qq.com/cgi-bin/netcheck");
    assert_eq!(url.host(), "www.qq.com");
    assert_eq!(
        url.port(),
        80,
        "a URL that names no port is the default one"
    );
    assert_eq!(url.path(), "/cgi-bin/netcheck");
    assert_eq!(url.url(), "http://www.qq.com/cgi-bin/netcheck");
}

#[test]
fn a_url_that_names_a_port_names_the_port() {
    let url = HttpUrlParser::new("http://1.2.3.4:8080/x");
    assert_eq!(url.host(), "1.2.3.4");
    assert_eq!(url.port(), 8080);
    assert_eq!(url.path(), "/x");
}

#[test]
fn a_url_with_no_path_goes_to_the_root() {
    // `path_ = url_.substr(schema_end)`, which is empty, so it becomes `/`
    let url = HttpUrlParser::new("http://1.2.3.4");
    assert_eq!(url.host(), "1.2.3.4");
    assert_eq!(url.path(), "/");

    // a path of just the slash is the same thing
    assert_eq!(HttpUrlParser::new("http://1.2.3.4/").path(), "/");
}

#[test]
fn the_scheme_is_read_without_its_case() {
    // `ci_find_substr` — only the scheme is matched case insensitively; the
    // host and the path keep theirs
    let url = HttpUrlParser::new("HTTP://HOST:8080/PATH");
    assert_eq!(url.host(), "HOST");
    assert_eq!(url.port(), 8080);
    assert_eq!(url.path(), "/PATH");
}

#[test]
fn a_url_without_the_scheme_is_not_read_at_all() {
    // `schema_start == 0`, so `Parse()` gives up before it fills anything in:
    // even the path stays empty, which is how a caller sees it
    let url = HttpUrlParser::new("www.qq.com/cgi-bin/netcheck");
    assert_eq!(url.host(), "");
    assert_eq!(url.path(), "");
    assert_eq!(url.url(), "www.qq.com/cgi-bin/netcheck");

    // the scheme is read from the trimmed URL, so a space in front of it is no
    // reason to give up — but another scheme is
    assert_eq!(
        HttpUrlParser::new(" http://www.qq.com/x").host(),
        "www.qq.com"
    );
    assert_eq!(HttpUrlParser::new("https://www.qq.com/x").host(), "");
}

#[test]
fn the_scheme_and_nothing_else_is_not_a_url() {
    // `schema_start >= url_.length()`
    let url = HttpUrlParser::new("http://");
    assert_eq!(url.host(), "");
    assert_eq!(url.port(), 80);
    assert_eq!(url.path(), "");
}

#[test]
fn the_slash_the_search_starts_past_is_the_host() {
    // `ci_find_substr(url_, "/", schema_start + 1)` starts one byte past the
    // scheme, so the `/` standing right behind it is not the one it finds:
    // `http:///x` reads the host as `/x`, and `http:////x` — one slash further
    // on — reads it as `/`
    let url = HttpUrlParser::new("http:///x");
    assert_eq!(url.host(), "/x");
    assert_eq!(url.port(), 80);
    assert_eq!(url.path(), "/");

    let url = HttpUrlParser::new("http:////x");
    assert_eq!(url.host(), "/");
    assert_eq!(url.port(), 80);
    assert_eq!(url.path(), "/x");
}

#[test]
fn the_user_info_of_a_url_is_not_the_host() {
    // `user:pwd@host` — the host is what comes after the `@`, and the port is
    // read after it, not after the colon of the password
    let url = HttpUrlParser::new("http://user:pwd@1.2.3.4:8080/x");
    assert_eq!(url.host(), "1.2.3.4");
    assert_eq!(url.port(), 8080);
    assert_eq!(url.path(), "/x");
}

#[test]
fn the_colons_of_an_ipv6_host_are_not_the_port() {
    // `ci_find_substr(hoststr, ":", host_start)` finds the first colon, which
    // in `[::1]:8080` is one of the address's — the C++ reads `[` as the host
    // and leaves the port at 80, so a bracketed host is skipped here
    let url = HttpUrlParser::new("http://[::1]:8080/x");
    assert_eq!(url.host(), "[::1]");
    assert_eq!(url.port(), 8080);
    assert_eq!(url.path(), "/x");

    // and an address that names no port is the default one, not the `1` of
    // the address
    let url = HttpUrlParser::new("http://[::1]/x");
    assert_eq!(url.host(), "[::1]");
    assert_eq!(url.port(), 80);
}

#[test]
fn a_url_that_ends_in_a_colon_names_no_port() {
    // `hoststr.length() - 1 == portstart`
    let url = HttpUrlParser::new("http://1.2.3.4:/x");
    assert_eq!(url.host(), "1.2.3.4");
    assert_eq!(url.port(), 80);
}

#[test]
fn a_port_that_did_not_read_as_a_number_is_the_default_one() {
    assert_eq!(HttpUrlParser::new("http://1.2.3.4:abc/x").port(), 80);
    assert_eq!(HttpUrlParser::new("http://1.2.3.4:0/x").port(), 80);
    // `atoi` skips the whitespace in front of the digits, which the URL may
    // carry around a host that was trimmed anyway
    assert_eq!(HttpUrlParser::new("http://1.2.3.4: 8080/x").port(), 8080);
}

#[test]
fn a_port_the_c_reads_as_negative_is_cast_and_not_dropped() {
    // `(uint16_t)atoi("-1")`: the cast wraps, so a sign a URL had no business
    // carrying is a port and not a number that did not read
    assert_eq!(HttpUrlParser::new("http://1.2.3.4:-1/x").port(), 65535);
    // and a `+` is the number it stands in front of
    assert_eq!(HttpUrlParser::new("http://1.2.3.4:+8080/x").port(), 8080);
}

#[test]
fn a_port_that_does_not_fit_in_sixteen_bits_is_truncated() {
    // `(uint16_t)atoi(...)`, which wraps: 65536 is 0, and 0 is the default
    assert_eq!(HttpUrlParser::new("http://1.2.3.4:65536/x").port(), 80);
    assert_eq!(HttpUrlParser::new("http://1.2.3.4:65537/x").port(), 1);
    // and one too long even for the C's `long` is `strtol`'s `LONG_MAX` —
    // `65535` once it is cast, and not the `0` that becomes the default
    assert_eq!(
        HttpUrlParser::new("http://1.2.3.4:99999999999999999999/x").port(),
        65535
    );
}

#[test]
fn the_url_and_the_host_it_was_read_from_are_trimmed() {
    // `strutil::Trim(url_)` in the constructor, and `strutil::Trim(host_)`
    let url = HttpUrlParser::new(" \thttp:// 1.2.3.4 :8080/x \r\n");
    assert_eq!(url.url(), "http:// 1.2.3.4 :8080/x");
    assert_eq!(url.host(), "1.2.3.4");
    assert_eq!(url.port(), 8080);
    assert_eq!(url.path(), "/x");

    // a URL of nothing but whitespace is one with no scheme
    assert_eq!(HttpUrlParser::new(" \t\r\n").host(), "");
    assert_eq!(HttpUrlParser::new("").host(), "");
}

#[test]
fn a_host_that_is_not_ascii_is_read_all_the_same() {
    // `ci_find_substr(url_, "/", schema_start + 1)` starts one byte past the
    // scheme, and a host need not be ASCII: `é` is two bytes, so the byte the
    // search starts at is in the middle of a character and not a boundary a
    // slice may be taken at
    let url = HttpUrlParser::new("http://éxample.com/x");
    assert_eq!(url.host(), "éxample.com");
    assert_eq!(url.port(), 80);
    assert_eq!(url.path(), "/x");

    // the port and the path are read out of it the same way
    let url = HttpUrlParser::new("http://éxample.com:8080/x");
    assert_eq!(url.host(), "éxample.com");
    assert_eq!(url.port(), 8080);
    assert_eq!(url.path(), "/x");
}

#[test]
fn a_parser_is_the_url_it_was_given() {
    let url = HttpUrlParser::new("http://1.2.3.4:8080/x");
    let same = url.clone();
    assert_eq!(same, url);
    assert_eq!(same.host(), url.host());
    assert_eq!(same.port(), url.port());
    assert_eq!(same.path(), url.path());
    assert!(
        format!("{url:?}").contains("HttpUrlParser"),
        "{}",
        format!("{url:?}")
    );
}
