//! `mars/sdt/src/checkimpl/http_url_parser.h` — the URL of an HTTP check, as a
//! host, a port and a path.
//!
//! `HttpUrlParser(url)` parses in its constructor, http and nothing else, and a
//! URL it could not read leaves an empty [`HttpUrlParser::host`] — which, with a
//! [`HttpUrlParser::path`] that is still empty, is all a caller can see of that.
//! The C++ keeps the `bool` `Parse()` returns to itself; so does this.
//!
//! `strutil::Trim` is here as the private `trim`: this crate has no `mars-comm` to ask, and
//! the C++ trims the URL it was given and the host it read out of it.

/// `kHttpSchema` — the scheme the parser understands, and the only one it does.
const HTTP_SCHEME: &str = "http://";
/// The port of a URL that named none, of one whose port is `0` or unreadable,
/// and of one that names no host at all: what `port_(80)` starts out as.
const DEFAULT_PORT: u16 = 80;

/// `strutil::Trim` — the ASCII whitespace the C++ strips off both ends.
const WHITESPACE: &[u8] = b" \t\n\x0b\x0c\r";

/// `strutil::Trim`.
fn trim(s: &str) -> &str {
    let bytes = s.as_bytes();
    let start = bytes
        .iter()
        .position(|byte| !WHITESPACE.contains(byte))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !WHITESPACE.contains(byte))
        .map_or(start, |at| at + 1);
    &s[start..end]
}

/// `atoi` and the C++'s `(uint16_t)` cast: the digits at the front of `text`,
/// truncated to sixteen bits, and `0` when there are none — which the parser
/// turns into [`DEFAULT_PORT`].
fn atoi(text: &str) -> u16 {
    let digits: String = text
        .chars()
        .skip_while(|char| char.is_ascii_whitespace())
        .take_while(|char| char.is_ascii_digit())
        .collect();
    digits.parse::<u64>().unwrap_or(0) as u16
}

/// `HttpUrlParser` — one URL, split into the three things an HTTP check needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpUrlParser {
    url: String,
    host: String,
    port: u16,
    path: String,
}

impl HttpUrlParser {
    /// `HttpUrlParser(url)` — the URL trimmed and parsed: every field is filled
    /// in by the time this returns, the way the C++ constructor's `Parse()` does.
    pub fn new(url: &str) -> Self {
        let mut parser = Self {
            url: trim(url).to_owned(),
            host: String::new(),
            port: DEFAULT_PORT,
            path: String::new(),
        };
        parser.parse();
        parser
    }

    /// `Host()` — empty for a URL that did not parse.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// `Port()` — `80` for a URL that named none.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// `Path()` — `/` for a URL that named none, and still empty for one that
    /// did not parse at all.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The URL as it was parsed, i.e. trimmed: `url_` of the C++, which the
    /// parser keeps because it is what the host and the path were read out of.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// `Parse()` — `false` for a URL that is not an http one, in which case
    /// nothing is filled in.
    fn parse(&mut self) -> bool {
        // `ci_find_substr(url_, kHttpSchema, 0)`: the scheme has to be at the
        // very front, and its case does not matter
        let scheme = self.url.as_bytes().get(..HTTP_SCHEME.len());
        if !scheme.is_some_and(|scheme| scheme.eq_ignore_ascii_case(HTTP_SCHEME.as_bytes())) {
            return false;
        }
        let scheme_start = HTTP_SCHEME.len();
        // `schema_start >= url_.length()`: the scheme and nothing else
        if scheme_start >= self.url.len() {
            return false;
        }

        // `ci_find_substr(url_, "/", schema_start + 1)` — the first `/` after
        // the scheme ends the host
        let scheme_end = self.url[scheme_start + 1..]
            .find('/')
            .map_or(self.url.len(), |at| scheme_start + 1 + at);
        let hoststr = trim(&self.url[scheme_start..scheme_end]);

        // `ci_find_substr(hoststr, "@", 0)` — `user:pwd@host`, and the host is
        // what comes after the `@`
        let host_start = hoststr.find('@').map_or(0, |at| at + 1);

        // `ci_find_substr(hoststr, ":", host_start)` — a port, unless the URL
        // ends in a colon and names none
        let (host, port) = match hoststr[host_start..].find(':') {
            None => (hoststr[host_start..].to_owned(), DEFAULT_PORT),
            Some(at) => {
                let port_start = host_start + at;
                let host = hoststr[host_start..port_start].to_owned();
                let port = if port_start + 1 == hoststr.len() {
                    DEFAULT_PORT
                } else {
                    atoi(&hoststr[port_start + 1..])
                };
                (host, port)
            }
        };
        self.host = trim(&host).to_owned();
        self.port = if port == 0 { DEFAULT_PORT } else { port };

        // `path_ = url_.substr(schema_end)`, and `/` when that is empty — a URL
        // with no path goes to the root
        self.path = self.url[scheme_end..].to_owned();
        if self.path.is_empty() {
            self.path = String::from("/");
        }

        !self.host.is_empty()
    }
}
