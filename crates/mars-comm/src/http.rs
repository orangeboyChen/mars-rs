//! `mars/comm/http.h` — the http the short link speaks.
//!
//! The short link is one http request on a socket of its own, and this is the
//! little http that writes it and reads the answer: a request line or a status
//! line, the header fields, and a body that is either a `Content-Length` of
//! bytes or a stream of chunks.
//!
//! What the C++ reads and writes through an `AutoBuffer` and a heap of
//! out-parameters is a value here: [`RequestLine::parse`] and
//! [`StatusLine::parse`] answer `Option`, [`HeaderFields::range`] is an
//! `Option<(i64, i64)>`, and [`Builder::header_to_buffer`] hands the bytes back
//! instead of writing them into a buffer it was given.
//!
//! Three things of the C++ are not here. Its `IBlockBodyProvider` and
//! `IStreamBodyProvider` are one [`Body`]: a block body is bytes, and a stream
//! body is bytes someone framed with [`chunk_header`] and [`CHUNK_EOF`], which
//! is all that interface really was. Its `BodyReceiver`, which a caller
//! subclasses to be told about the body, is the [`Vec`] a [`Parser`] collects
//! the body in — [`Parser::body`] is what every caller wanted from it. And its
//! `URLFactory` and `StringBody` have no caller in mars at all.
//!
//! A header that is not UTF-8 is read with [`String::from_utf8_lossy`]: the C++
//! keeps the raw bytes in a `std::string`, and no header of mars' is anything
//! but ASCII.

use std::fmt;

use crate::strutil;

/// `KStringCRLF` — what ends every line of the head.
pub const CRLF: &str = "\r\n";
/// What ends the head: an empty line.
const CRLF_CRLF: &str = "\r\n\r\n";

/// How long a first line may grow before the C++ gives up on it: `8 * 1024`.
pub const MAX_FIRST_LINE: usize = 8 * 1024;
/// How long the head may grow before the C++ gives up on it: `128 * 1024`.
pub const MAX_HEADER_FIELDS: usize = 128 * 1024;
/// `kMaxContentLength` — how big a body may be: 4g.
pub const MAX_CONTENT_LENGTH: u64 = 4 * 1024 * 1024 * 1024;
/// `kMaxChunkLength` — how big one chunk may be: 4g.
pub const MAX_CHUNK_LENGTH: u64 = 4 * 1024 * 1024 * 1024;
/// `KDefaultKeepAliveTimeout` — what a keep-alive without a timeout of its own
/// gets: five seconds.
pub const DEFAULT_KEEP_ALIVE_TIMEOUT: u32 = 5;

/// `IStreamBodyProvider::AppendHeader` — the size line of one chunk: the length
/// in hexadecimal, `CRLF` and all.
pub fn chunk_header(len: usize) -> String {
    format!("{len:x}{CRLF}")
}

/// `IStreamBodyProvider::AppendTail` — what ends the bytes of one chunk.
pub const CHUNK_TAIL: &str = CRLF;
/// `IStreamBodyProvider::EofData` — the last chunk, which is the one of no
/// bytes.
pub const CHUNK_EOF: &str = "0\r\n\r\n";

/// `THttpVersion`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    /// `kVersion_0_9`
    V0_9,
    /// `kVersion_1_0` — what a [`RequestLine`] and a [`StatusLine`] start with.
    V1_0,
    /// `kVersion_1_1`
    V1_1,
    /// `kVersion_2_0`
    V2_0,
    /// `kVersion_Unknow` — what matches none of the strings.
    Unknown,
}

impl Version {
    /// `kHttpVersionString[version]` — what goes on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V0_9 => "HTTP/0.9",
            Self::V1_0 => "HTTP/1.0",
            Self::V1_1 => "HTTP/1.1",
            Self::V2_0 => "HTTP/2",
            Self::Unknown => "version_unknown",
        }
    }

    /// `__GetHttpVersion` — the version one token of the first line is.
    pub fn parse(text: &str) -> Self {
        [Self::V0_9, Self::V1_0, Self::V1_1, Self::V2_0]
            .into_iter()
            .find(|version| version.as_str() == text)
            .unwrap_or(Self::Unknown)
    }
}

impl Default for Version {
    /// `kVersion_1_0` — what a [`RequestLine`] and a [`StatusLine`] start with.
    #[allow(clippy::derivable_impls)]
    fn default() -> Self {
        Self::V1_0
    }
}

/// `RequestLine::THttpMethod`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `kUnknown` — what matches none of the strings, and what a request line
    /// therefore cannot be made of.
    Unknown,
    /// `kGet` — what a [`RequestLine`] starts with.
    Get,
    /// `kPost` — what the short link sends.
    Post,
    /// `kOptions`
    Options,
    /// `kHead`
    Head,
    /// `kPut`
    Put,
    /// `kDelete`
    Delete,
    /// `kTrace`
    Trace,
    /// `kConnect`
    Connect,
}

impl Method {
    /// `kHttpMethodString[method]` — what goes on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "UNKNOWN",
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Options => "OPTIONS",
            Self::Head => "HEAD",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Trace => "TRACE",
            Self::Connect => "CONNECT",
        }
    }

    /// The method one token of the first line is.
    pub fn parse(text: &str) -> Self {
        [
            Self::Get,
            Self::Post,
            Self::Options,
            Self::Head,
            Self::Put,
            Self::Delete,
            Self::Trace,
            Self::Connect,
        ]
        .into_iter()
        .find(|method| method.as_str() == text)
        .unwrap_or(Self::Unknown)
    }
}

impl Default for Method {
    /// `kGet` — what a [`RequestLine`] starts with.
    #[allow(clippy::derivable_impls)]
    fn default() -> Self {
        Self::Get
    }
}

/// `TCsMode` — whether what is being written or read is a request or an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsMode {
    /// `kRequest`
    Request,
    /// `kRespond` — what a [`Parser`] starts with, which is the C++'s
    /// constructor's `csmode_(kRespond)`.
    Respond,
}

impl Default for CsMode {
    /// `kRespond` — what a [`Parser`] starts with.
    #[allow(clippy::derivable_impls)]
    fn default() -> Self {
        Self::Respond
    }
}

/// `RequestLine` — the first line of a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestLine {
    /// `Method()`
    pub method: Method,
    /// `Url()`
    pub url: String,
    /// `Version()`
    pub version: Version,
}

impl RequestLine {
    /// `RequestLine(method, url, version)`.
    pub fn new(method: Method, url: &str, version: Version) -> Self {
        Self {
            method,
            url: url.to_string(),
            version,
        }
    }

    /// `FromString` — the line as it came off the wire, `CRLF` and all.
    /// `None` is what the C++ answers `false` for: a line that is not
    /// CRLF-terminated, fewer than three tokens, an unknown method or an
    /// unknown version.
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.split_once(CRLF)?.0;
        let tokens = strutil::split_token(line, " ");
        if tokens.len() < 3 {
            return None;
        }
        let method = Method::parse(tokens[0]);
        if method == Method::Unknown {
            return None;
        }
        let version = Version::parse(tokens[2]);
        if version == Version::Unknown {
            return None;
        }
        Some(Self::new(method, tokens[1], version))
    }
}

impl Default for RequestLine {
    fn default() -> Self {
        Self::new(Method::Get, "", Version::V1_0)
    }
}

impl fmt::Display for RequestLine {
    /// `ToString`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {}{}",
            self.method.as_str(),
            self.url,
            self.version.as_str(),
            CRLF
        )
    }
}

/// `StatusLine` — the first line of an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLine {
    /// `Version()`
    pub version: Version,
    /// `StatusCode()` — the C++'s `int`, which is what the short link compares
    /// against 200.
    pub status_code: i32,
    /// `ReasonPhrase()`
    pub reason_phrase: String,
}

impl StatusLine {
    /// `StatusLine(version, statuscode, reasonphrase)`.
    pub fn new(version: Version, status_code: i32, reason_phrase: &str) -> Self {
        Self {
            version,
            status_code,
            reason_phrase: reason_phrase.to_string(),
        }
    }

    /// `FromString` — the line as it came off the wire, `CRLF` and all.
    ///
    /// A reason phrase of more than one word is dropped: the C++ takes it only
    /// when the line is exactly three tokens, so `HTTP/1.1 404 Not Found` comes
    /// back with no reason phrase at all.
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.split_once(CRLF)?.0;
        let tokens = strutil::split_token(line, " ");
        if tokens.len() < 2 {
            return None;
        }
        let version = Version::parse(tokens[0]);
        if version == Version::Unknown {
            return None;
        }
        let reason_phrase = if tokens.len() == 3 { tokens[2] } else { "" };
        Some(Self::new(version, to_status_code(tokens[1]), reason_phrase))
    }
}

impl Default for StatusLine {
    fn default() -> Self {
        Self::new(Version::V1_0, 0, "")
    }
}

impl fmt::Display for StatusLine {
    /// `ToString`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {}{}",
            self.version.as_str(),
            self.status_code,
            self.reason_phrase,
            CRLF
        )
    }
}

/// `HeaderFields::KStringHost`.
pub const HOST: &str = "Host";
/// `HeaderFields::KStringAccept`.
pub const ACCEPT: &str = "Accept";
/// `HeaderFields::KStringUserAgent`.
pub const USER_AGENT: &str = "User-Agent";
/// `HeaderFields::KStringCacheControl`.
pub const CACHE_CONTROL: &str = "Cache-Control";
/// `HeaderFields::KStringConnection`.
pub const CONNECTION: &str = "Connection";
/// `HeaderFields::kStringProxyConnection`.
pub const PROXY_CONNECTION: &str = "Proxy-Connection";
/// `HeaderFields::kStringProxyAuthorization`.
pub const PROXY_AUTHORIZATION: &str = "Proxy-Authorization";
/// `HeaderFields::KStringContentType`.
pub const CONTENT_TYPE: &str = "Content-Type";
/// `HeaderFields::KStringContentLength`.
pub const CONTENT_LENGTH: &str = "Content-Length";
/// `HeaderFields::KStringTransferEncoding`.
pub const TRANSFER_ENCODING: &str = "Transfer-Encoding";
/// `HeaderFields::kStringContentEncoding`.
pub const CONTENT_ENCODING: &str = "Content-Encoding";
/// `HeaderFields::KStringAcceptEncoding`.
pub const ACCEPT_ENCODING: &str = "Accept-Encoding";
/// `HeaderFields::KStringContentRange`.
pub const CONTENT_RANGE: &str = "Content-Range";
/// `HeaderFields::KStringRange`.
pub const RANGE: &str = "Range";
/// `HeaderFields::KStringLocation`.
pub const LOCATION: &str = "Location";
/// `HeaderFields::KStringReferer`.
pub const REFERER: &str = "Referer";
/// `HeaderFields::kStringServer`.
pub const SERVER: &str = "Server";
/// `HeaderFields::KStringKeepalive` — the header the timeout is read from,
/// which is not the one that says whether the connection is kept alive.
pub const KEEP_ALIVE: &str = "Keep-Alive";

/// `KStringMicroMessenger` — the user agent of a short link.
pub const USER_AGENT_MICRO_MESSAGE: &str = "MicroMessenger Client";
const CHUNKED: &str = "chunked";
const CLOSE: &str = "close";
const KEEPALIVE: &str = "Keep-Alive";
const ACCEPT_ALL: &str = "*/*";
const NO_CACHE: &str = "no-cache";
const OCTET_STREAM: &str = "application/octet-stream";
const DEFLATE: &str = "deflate";
const GZIP: &str = "gzip";
const KEEP_ALIVE_TIMEOUT: &str = "timeout=";

/// One header field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// The name, kept the way it was first written: a field set twice keeps the
    /// name of the first time.
    pub name: String,
    /// The value.
    pub value: String,
}

impl Header {
    /// A field of `name` and `value`.
    pub fn new(name: &str, value: &str) -> Self {
        Self {
            name: name.to_string(),
            value: value.to_string(),
        }
    }
}

/// `HeaderFields` — the head of a request or an answer.
///
/// The C++ keeps them in a `std::map` with a case-insensitive comparator, which
/// is a list here: the names are compared the same way, and what the map's
/// order gave it — a head that comes out sorted by name — is what insertion
/// order gives as well for every head mars writes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderFields {
    headers: Vec<Header>,
}

impl HeaderFields {
    /// `HeaderFields()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `HeaderFiled(name, value)` — a field that is not there yet is added, and
    /// one that is keeps its name and gets the new value.
    pub fn set(&mut self, name: &str, value: &str) {
        match self
            .headers
            .iter_mut()
            .find(|header| header.name.eq_ignore_ascii_case(name))
        {
            Some(header) => header.value = value.to_string(),
            None => self.headers.push(Header::new(name, value)),
        }
    }

    /// `Manipulate(name, value)` — an empty value takes the field away, which
    /// is how a caller that was handed a field to change takes one out.
    pub fn manipulate(&mut self, name: &str, value: &str) {
        if strutil::trim(value).is_empty() {
            self.remove(name);
        } else {
            self.set(name, value);
        }
    }

    /// What [`HeaderFields::manipulate`] does with an empty value.
    pub fn remove(&mut self, name: &str) {
        self.headers
            .retain(|header| !header.name.eq_ignore_ascii_case(name));
    }

    /// `HeaderField(key)` — the value of a field, whichever way its name was
    /// written.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
    }

    /// `GetAsList()` — the fields, in the order they were set.
    pub fn headers(&self) -> &[Header] {
        &self.headers
    }

    /// `GetHeaders().size()` — how many fields there are.
    pub fn len(&self) -> usize {
        self.headers.len()
    }

    /// Whether there are no fields at all, which is what the C++'s
    /// `ToString().empty()` asks.
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty()
    }

    /// `MakeContentLength` and `HeaderFiled` in one: `Content-Length`.
    pub fn set_content_length(&mut self, len: u64) {
        self.set(CONTENT_LENGTH, &len.to_string());
    }

    /// `MakeTransferEncodingChunked` — `Transfer-Encoding: chunked`.
    pub fn set_chunked(&mut self) {
        self.set(TRANSFER_ENCODING, CHUNKED);
    }

    /// `MakeConnectionClose` — what a short link asks for.
    pub fn set_connection_close(&mut self) {
        self.set(CONNECTION, CLOSE);
    }

    /// `MakeConnectionKeepalive`.
    pub fn set_connection_keepalive(&mut self) {
        self.set(CONNECTION, KEEPALIVE);
    }

    /// `MakeAcceptAll`.
    pub fn set_accept_all(&mut self) {
        self.set(ACCEPT, ACCEPT_ALL);
    }

    /// `MakeAcceptEncodingDefalte`.
    pub fn set_accept_encoding_deflate(&mut self) {
        self.set(ACCEPT_ENCODING, DEFLATE);
    }

    /// `MakeAcceptEncodingGzip`.
    pub fn set_accept_encoding_gzip(&mut self) {
        self.set(ACCEPT_ENCODING, GZIP);
    }

    /// `MakeCacheControlNoCache`.
    pub fn set_cache_control_no_cache(&mut self) {
        self.set(CACHE_CONTROL, NO_CACHE);
    }

    /// `MakeContentTypeOctetStream`.
    pub fn set_content_type_octet_stream(&mut self) {
        self.set(CONTENT_TYPE, OCTET_STREAM);
    }

    /// `MakeUserAgentMicroMessage`.
    pub fn set_user_agent_micro_message(&mut self) {
        self.set(USER_AGENT, USER_AGENT_MICRO_MESSAGE);
    }

    /// `IsTransferEncodingChunked()`.
    pub fn is_chunked(&self) -> bool {
        self.is_value(TRANSFER_ENCODING, CHUNKED)
    }

    /// `IsConnectionClose()`.
    pub fn is_connection_close(&self) -> bool {
        self.is_value(CONNECTION, CLOSE)
    }

    /// `IsConnectionKeepAlive()` — what the short link asks an answer about
    /// before it keeps the socket.
    pub fn is_connection_keep_alive(&self) -> bool {
        self.is_value(CONNECTION, KEEPALIVE)
    }

    /// `ContentLength()` — `0` when there is no such field.
    pub fn content_length(&self) -> u64 {
        self.get(CONTENT_LENGTH).map(to_u64).unwrap_or(0)
    }

    /// `KeepAliveTimeout()` — how long the peer will keep the socket, in
    /// seconds; [`DEFAULT_KEEP_ALIVE_TIMEOUT`] when it did not say, or when
    /// what it said is not a timeout between nothing and a minute.
    ///
    /// The timeout is read from `Keep-Alive`, and only when there is a
    /// `Connection` field at all. The C++ skips `sizeof(const char*)` — 8 —
    /// characters to get past `timeout=`, which is how long `timeout=` is.
    pub fn keep_alive_timeout(&self) -> u32 {
        if self.get(CONNECTION).is_none() {
            return DEFAULT_KEEP_ALIVE_TIMEOUT;
        }
        let Some(alive) = self.get(KEEP_ALIVE) else {
            return DEFAULT_KEEP_ALIVE_TIMEOUT;
        };
        if alive.is_empty() || !alive.contains(KEEP_ALIVE_TIMEOUT) {
            return DEFAULT_KEEP_ALIVE_TIMEOUT;
        }
        let timeout = alive
            .split(',')
            .find_map(|token| token.split_once(KEEP_ALIVE_TIMEOUT))
            .map(|(_, value)| to_u64(value))
            .unwrap_or(0);
        // `timeout > 0 && timeout < 60`, and the C++ gives up on the whole
        // header as soon as it has read one timeout out of it
        u32::try_from(timeout)
            .ok()
            .filter(|timeout| (1..60).contains(timeout))
            .unwrap_or(DEFAULT_KEEP_ALIVE_TIMEOUT)
    }

    /// `Range(start, end)` — `bytes=from-to`, which is what a caller that wants
    /// part of an answer writes.
    pub fn range(&self) -> Option<(i64, i64)> {
        let range = self.get(RANGE)?;
        let bytes = strutil::trim(range.strip_prefix("bytes=")?);
        let (from, to) = bytes.split_once('-')?;
        Some((to_i64(from), to_i64(to)))
    }

    /// `ContentRange(start, end, total)` — `bytes from-to/total`.
    pub fn content_range(&self) -> Option<ContentRange> {
        Self::content_range_of(self.get(CONTENT_RANGE)?)
    }

    /// `ContentRange(line, start, end, total)` — the same, of a line that did
    /// not come out of a head: `Content-Range: bytes 0-102400/102399`.
    pub fn content_range_of(line: &str) -> Option<ContentRange> {
        let bytes = strutil::trim(line.strip_prefix("bytes ")?);
        let (from, rest) = bytes.split_once('-')?;
        let (to, total) = rest.split_once('/')?;
        Some(ContentRange {
            start: to_u64(from),
            end: to_u64(to),
            total: to_u64(total),
        })
    }

    /// `__ParserHeaders` — the fields of one block, `CRLF`-separated and
    /// `CRLF CRLF`-ended.
    ///
    /// A line with no colon in it becomes a field of itself, which is what the
    /// C++'s own arithmetic does with one; a line that is nothing but colons is
    /// skipped, and so is the empty line the head ends with.
    pub fn parse(&mut self, block: &str) {
        for line in block.split(CRLF) {
            if line.chars().all(|c| c == ':') {
                continue;
            }
            let (name, value) = match line.find(':') {
                Some(colon) if line.len() > colon + 1 => (&line[..colon], &line[colon + 1..]),
                Some(_) => continue,
                None => (line, line),
            };
            self.set(strutil::trim(name), strutil::trim(value));
        }
    }

    fn is_value(&self, name: &str, value: &str) -> bool {
        self.get(name)
            .is_some_and(|found| found.eq_ignore_ascii_case(value))
    }
}

impl fmt::Display for HeaderFields {
    /// `ToString` — `name: value` per line, in the order they were set.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for header in &self.headers {
            write!(f, "{}: {}{}", header.name, header.value, CRLF)?;
        }
        Ok(())
    }
}

/// What a `Content-Range` says: which bytes these are, and how many there are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContentRange {
    /// The first byte.
    pub start: u64,
    /// The last byte.
    pub end: u64,
    /// How many bytes there are in all.
    pub total: u64,
}

/// What a [`Builder`] or a [`Parser`] carries as the body: `IBlockBodyProvider`
/// and `IStreamBodyProvider` in one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// `IBlockBodyProvider` — the whole body at once, which is what gets a
    /// `Content-Length`.
    Block(Vec<u8>),
    /// `IStreamBodyProvider` — the body as chunks someone framed with
    /// [`chunk_header`], [`CHUNK_TAIL`] and [`CHUNK_EOF`], which is what gets a
    /// `Transfer-Encoding`.
    Chunks(Vec<u8>),
}

/// `Builder` — the request or the answer that goes out on the socket.
#[derive(Debug, Clone, Default)]
pub struct Builder {
    mode: CsMode,
    request: RequestLine,
    status: StatusLine,
    fields: HeaderFields,
    body: Option<Body>,
}

impl Builder {
    /// `Builder(csmode)`.
    pub fn new(mode: CsMode) -> Self {
        Self {
            mode,
            ..Self::default()
        }
    }

    /// `csmode_`.
    pub fn mode(&self) -> CsMode {
        self.mode
    }

    /// `Request()`.
    pub fn request(&self) -> &RequestLine {
        &self.request
    }

    /// `Request()` — what the caller writes the method, the url and the version
    /// into.
    pub fn request_mut(&mut self) -> &mut RequestLine {
        &mut self.request
    }

    /// `Status()`.
    pub fn status(&self) -> &StatusLine {
        &self.status
    }

    /// `Status()` — what the caller writes the status code into.
    pub fn status_mut(&mut self) -> &mut StatusLine {
        &mut self.status
    }

    /// `Fields()`.
    pub fn fields(&self) -> &HeaderFields {
        &self.fields
    }

    /// `Fields()` — what the caller writes the head into.
    pub fn fields_mut(&mut self) -> &mut HeaderFields {
        &mut self.fields
    }

    /// `BlockBody(body)` / `StreamBody(body)` — what goes after the head.
    ///
    /// The C++ manages the provider it is handed behind a pointer and can be
    /// asked for it again; here the body is owned, and [`Builder::to_buffer`]
    /// takes it.
    pub fn set_body(&mut self, body: Body) {
        self.body = Some(body);
    }

    /// `BlockBody()` / `StreamBody()`.
    pub fn body(&self) -> Option<&Body> {
        self.body.as_ref()
    }

    /// `HeaderToBuffer` — the first line, the head, and the empty line that
    /// ends it. `None` when there is no head to write, which is what the C++
    /// answers `false` for.
    pub fn header_to_buffer(&self) -> Option<Vec<u8>> {
        let first_line = match self.mode {
            CsMode::Request => self.request.to_string(),
            CsMode::Respond => self.status.to_string(),
        };
        if first_line.is_empty() || self.fields.is_empty() {
            return None;
        }
        let mut buffer = first_line.into_bytes();
        buffer.extend_from_slice(self.fields.to_string().as_bytes());
        buffer.extend_from_slice(CRLF.as_bytes());
        Some(buffer)
    }

    /// `HttpToBuffer` — the whole thing, head and body.
    ///
    /// A block body gets a `Content-Length` written for it and is then taken:
    /// the C++'s `FillData` moves it into the buffer and leaves the provider
    /// empty. A block body of no bytes at all writes nothing — not even the
    /// head — which is the C++'s own answer for one.
    pub fn to_buffer(&mut self) -> Option<Vec<u8>> {
        match self.body.take() {
            Some(Body::Block(body)) if !body.is_empty() => {
                self.fields.set_content_length(body.len() as u64);
                let mut buffer = self.header_to_buffer()?;
                buffer.extend_from_slice(&body);
                Some(buffer)
            }
            Some(Body::Block(_)) => Some(Vec::new()),
            Some(Body::Chunks(chunks)) => {
                self.fields.set_chunked();
                let mut buffer = self.header_to_buffer()?;
                buffer.extend_from_slice(&chunks);
                Some(buffer)
            }
            None => self.header_to_buffer(),
        }
    }
}

/// `Parser::TRecvStatus` — how far a [`Parser`] got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecvStatus {
    /// `kStart` — nothing has come in yet.
    #[default]
    Start,
    /// `kFirstLine` — the first line is not whole yet.
    FirstLine,
    /// `kFirstLineError` — a first line that is not one.
    FirstLineError,
    /// `kHeaderFields` — the head is not whole yet.
    HeaderFields,
    /// `kHeaderFieldsError` — a head that never ended.
    HeaderFieldsError,
    /// `kBody` — the body is coming.
    Body,
    /// `kBodyError` — a body that is not the one the head said.
    BodyError,
    /// `kEnd` — the answer is whole.
    End,
}

impl RecvStatus {
    /// `Error()`.
    pub fn is_error(self) -> bool {
        matches!(
            self,
            Self::FirstLineError | Self::HeaderFieldsError | Self::BodyError
        )
    }

    /// `Success()`.
    pub fn is_end(self) -> bool {
        matches!(self, Self::End)
    }

    /// `FirstLineReady()` — the C++ asks this as `kFirstLineError < status`,
    /// which is true of a head that failed to parse as well.
    pub fn is_first_line_ready(self) -> bool {
        matches!(
            self,
            Self::HeaderFields | Self::HeaderFieldsError | Self::Body | Self::BodyError | Self::End
        )
    }

    /// `FieldsReady()` — `kHeaderFieldsError < status`.
    pub fn is_fields_ready(self) -> bool {
        matches!(self, Self::Body | Self::BodyError | Self::End)
    }

    /// `BodyReady()` — `kBodyError < status`, which is [`RecvStatus::End`] and
    /// nothing else.
    pub fn is_body_ready(self) -> bool {
        matches!(self, Self::End)
    }
}

/// `Parser` — the answer that comes back off the socket.
///
/// The C++ is handed the bytes a `Recv` read and answers how many of them it
/// used; this one keeps the bytes it was given until it has used them, so the
/// caller hands over everything it read and reads [`Parser::recv_status`].
#[derive(Debug, Clone, Default)]
pub struct Parser {
    status: RecvStatus,
    mode: CsMode,
    request: RequestLine,
    status_line: StatusLine,
    fields: HeaderFields,
    body: Vec<u8>,
    /// What came in and has not been used yet.
    buffer: Vec<u8>,
    first_line_len: usize,
    header_len: usize,
}

impl Parser {
    /// `Parser()` — a parser of an answer, with nothing in it yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// `Recv(buffer, length)` — the bytes one read of the socket gave, which is
    /// the whole call: what is not used yet stays for the next one.
    ///
    /// A read of nothing is the peer hanging up, which is how a body with no
    /// `Content-Length` — the answer of a `Connection: close` — comes to an end.
    pub fn recv(&mut self, bytes: &[u8]) -> RecvStatus {
        if bytes.is_empty() {
            if self.fields.is_connection_close() && self.status == RecvStatus::Body {
                self.status = RecvStatus::End;
            }
            return self.status;
        }
        self.buffer.extend_from_slice(bytes);
        self.run(false)
    }

    /// `Recv(buffer, length, nullptr, true)` — the same, stopping as soon as
    /// the head is whole.
    pub fn recv_header_only(&mut self, bytes: &[u8]) -> RecvStatus {
        if bytes.is_empty() {
            return self.status;
        }
        self.buffer.extend_from_slice(bytes);
        self.run(true)
    }

    /// `RecvStatus()`.
    pub fn recv_status(&self) -> RecvStatus {
        self.status
    }

    /// `CsMode()` — whether what came in is a request or an answer, which is
    /// decided by the first line.
    pub fn mode(&self) -> CsMode {
        self.mode
    }

    /// `Request()`.
    pub fn request(&self) -> &RequestLine {
        &self.request
    }

    /// `Status()`.
    pub fn status(&self) -> &StatusLine {
        &self.status_line
    }

    /// `Fields()`.
    pub fn fields(&self) -> &HeaderFields {
        &self.fields
    }

    /// `Fields()` — what a caller that changes the head before it asks for the
    /// body wants.
    pub fn fields_mut(&mut self) -> &mut HeaderFields {
        &mut self.fields
    }

    /// `Body()` — the bytes of the body so far.
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// `Body().Length()`.
    pub fn body_len(&self) -> usize {
        self.body.len()
    }

    /// What came in and has not been used yet, which is [`Parser::body`] that
    /// is not in it and the head that is not parsed.
    pub fn buffered(&self) -> &[u8] {
        &self.buffer
    }

    /// `FirstLineLength()`.
    pub fn first_line_len(&self) -> usize {
        self.first_line_len
    }

    /// `HeaderLength()`.
    pub fn header_len(&self) -> usize {
        self.header_len
    }

    /// `Error()`.
    pub fn is_error(&self) -> bool {
        self.status.is_error()
    }

    /// `Success()`.
    pub fn is_success(&self) -> bool {
        self.status.is_end()
    }

    /// `FirstLineReady()`.
    pub fn is_first_line_ready(&self) -> bool {
        self.status.is_first_line_ready()
    }

    /// `FieldsReady()`.
    pub fn is_fields_ready(&self) -> bool {
        self.status.is_fields_ready()
    }

    /// `BodyReady()`.
    pub fn is_body_ready(&self) -> bool {
        self.status.is_body_ready()
    }

    /// `BodyRecving()`.
    pub fn is_body_recving(&self) -> bool {
        self.status == RecvStatus::Body
    }

    fn run(&mut self, only_header: bool) -> RecvStatus {
        loop {
            let stop = match self.status {
                RecvStatus::Start | RecvStatus::FirstLine => self.first_line(),
                RecvStatus::HeaderFields => self.header_fields(only_header),
                RecvStatus::Body => self.read_body(),
                status => return status,
            };
            if stop {
                return self.status;
            }
        }
    }

    fn first_line(&mut self) -> bool {
        let Some(end) = find(&self.buffer, CRLF) else {
            self.status = if self.buffer.len() > MAX_FIRST_LINE {
                RecvStatus::FirstLineError
            } else {
                RecvStatus::FirstLine
            };
            return true;
        };

        let line_len = end + CRLF.len();
        let line = String::from_utf8_lossy(&self.buffer[..line_len]);
        let parsed = if line.starts_with("HTTP/") {
            StatusLine::parse(&line)
                .map(|status| {
                    self.status_line = status;
                    self.mode = CsMode::Respond;
                })
                .is_some()
        } else {
            RequestLine::parse(&line)
                .map(|request| {
                    self.request = request;
                    self.mode = CsMode::Request;
                })
                .is_some()
        };
        if !parsed {
            self.status = RecvStatus::FirstLineError;
            return true;
        }

        self.first_line_len = line_len;
        // `HTTP/1.1 401 Unauthorized\r\n\r\n`: a first line that is the whole
        // head, which is an answer with no fields in it
        if find(&self.buffer, CRLF_CRLF) == Some(end) {
            self.consume(line_len + CRLF.len());
            self.status = RecvStatus::Body;
        } else {
            self.consume(line_len);
            self.status = RecvStatus::HeaderFields;
        }
        false
    }

    fn header_fields(&mut self, only_header: bool) -> bool {
        let Some(end) = find(&self.buffer, CRLF_CRLF) else {
            if self.buffer.len() > MAX_HEADER_FIELDS {
                self.status = RecvStatus::HeaderFieldsError;
            }
            return true;
        };

        let header_len = end + CRLF_CRLF.len();
        let block = String::from_utf8_lossy(&self.buffer[..header_len]).to_string();
        self.fields.parse(&block);
        self.header_len = header_len;
        self.consume(header_len);
        self.status = RecvStatus::Body;
        // the head was all that was wanted, and it is whole now
        only_header
    }

    /// The body, one step: `true` when the parser has to go back to the
    /// caller for more bytes.
    fn read_body(&mut self) -> bool {
        if self.fields.is_chunked() {
            self.chunked_body()
        } else {
            self.whole_body()
        }
    }

    fn chunked_body(&mut self) -> bool {
        let Some(size_end) = find(&self.buffer, CRLF) else {
            return true;
        };
        // the size of a chunk is hexadecimal
        let size = to_u64_radix(
            strutil::trim(&String::from_utf8_lossy(&self.buffer[..size_end])),
            16,
        );
        if size > MAX_CHUNK_LENGTH {
            self.status = RecvStatus::BodyError;
            return true;
        }

        if size != 0 {
            let begin = size_end + CRLF.len();
            let end = begin + size as usize;
            if self.buffer.len() < end + CRLF.len() {
                return true;
            }
            if self.buffer[end] != b'\r' || self.buffer[end + 1] != b'\n' {
                self.status = RecvStatus::BodyError;
                return true;
            }
            self.body.extend_from_slice(&self.buffer[begin..end]);
            self.consume(end + CRLF.len());
            return false;
        }

        // the last chunk: a size of nothing, and then the trailer
        let trailer_begin = size_end + CRLF.len();
        if self.buffer.len() < trailer_begin + CRLF.len() {
            return true;
        }
        let Some(trailer_end) = find(&self.buffer[trailer_begin..], CRLF) else {
            return true;
        };
        self.consume(trailer_begin + trailer_end + CRLF.len());
        self.status = RecvStatus::End;
        true
    }

    fn whole_body(&mut self) -> bool {
        let content_length = self.fields.content_length();
        if content_length > MAX_CONTENT_LENGTH {
            self.status = RecvStatus::BodyError;
            return true;
        }

        let buffered = self.buffer.len() as u64;
        let have = self.body.len() as u64;
        let append = if self.fields.is_connection_close() && content_length == 0 {
            // a body the peer closes the socket at the end of has no length
            buffered
        } else if buffered + have <= content_length {
            buffered
        } else if have > content_length {
            // more body than the head said there was: the C++ reads a
            // `Content-Length` that underflowed and then fails the 4g check
            self.status = RecvStatus::BodyError;
            return true;
        } else {
            content_length - have
        };
        if append > MAX_CONTENT_LENGTH {
            self.status = RecvStatus::BodyError;
            return true;
        }

        self.body.extend_from_slice(&self.buffer[..append as usize]);
        self.consume(append as usize);
        if have + append == content_length {
            self.status = RecvStatus::End;
            return true;
        }
        self.buffer.is_empty()
    }

    fn consume(&mut self, len: usize) {
        self.buffer.drain(..len);
    }
}

/// `string_strnstr` — where `needle` starts in `buffer`, which is `None` when
/// it is not wholly in it.
fn find(buffer: &[u8], needle: &str) -> Option<usize> {
    buffer
        .windows(needle.len())
        .position(|window| window == needle.as_bytes())
}

/// What `strtol` reads into the C++'s `int` — the number at the front of a
/// token, and `0` when there is none.
fn to_status_code(text: &str) -> i32 {
    i32::try_from(to_i64(text)).unwrap_or(0)
}

/// What `strtol` reads out of a token: the number at the front of it, and `0`
/// when there is none.
fn to_i64(text: &str) -> i64 {
    let (sign, rest) = match text.as_bytes().first() {
        Some(b'-') => (-1, &text[1..]),
        Some(b'+') => (1, &text[1..]),
        _ => (1, text),
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let value = digits.parse::<i64>().unwrap_or(0);
    sign * value
}

/// What `strtoull` reads out of a token: the digits at the front of it.
fn to_u64(text: &str) -> u64 {
    to_u64_radix(text, 10)
}

/// What `strtoull(text, base)` reads: the digits of that base at the front of a
/// token — 16 for the size of a chunk, 10 everywhere else — and `0` when there
/// are none.
fn to_u64_radix(text: &str, radix: u32) -> u64 {
    let rest = text.trim_start().trim_start_matches('+');
    let digits: String = rest.chars().take_while(|c| c.is_digit(radix)).collect();
    u64::from_str_radix(&digits, radix).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_and_a_method_are_the_strings_on_the_wire() {
        assert_eq!(Version::V1_1.as_str(), "HTTP/1.1");
        assert_eq!(Version::V2_0.as_str(), "HTTP/2");
        assert_eq!(Version::default(), Version::V1_0, "what a line starts with");
        assert_eq!(Version::parse("HTTP/1.0"), Version::V1_0);
        assert_eq!(
            Version::parse("HTTP/3"),
            Version::Unknown,
            "and what is not one"
        );

        assert_eq!(Method::Post.as_str(), "POST");
        assert_eq!(Method::default(), Method::Get);
        assert_eq!(Method::parse("HEAD"), Method::Head);
        assert_eq!(Method::parse("PATCH"), Method::Unknown);
        assert_eq!(
            Method::parse("UNKNOWN"),
            Method::Unknown,
            "which is not a method a line may be made of"
        );
    }

    #[test]
    fn a_request_line_round_trips() {
        let line = RequestLine::new(Method::Post, "/cgi-bin/micromsg-bin", Version::V1_1);
        assert_eq!(line.to_string(), "POST /cgi-bin/micromsg-bin HTTP/1.1\r\n");
        assert_eq!(RequestLine::parse(&line.to_string()), Some(line));
    }

    #[test]
    fn a_line_that_is_not_one_is_none() {
        assert_eq!(RequestLine::parse("GET / HTTP/1.1"), None, "no CRLF");
        assert_eq!(RequestLine::parse("GET /\r\n"), None, "too few tokens");
        assert_eq!(
            RequestLine::parse("PATCH / HTTP/1.1\r\n"),
            None,
            "no such method"
        );
        assert_eq!(
            RequestLine::parse("GET / HTTP/9.9\r\n"),
            None,
            "no such version"
        );
    }

    #[test]
    fn a_status_line_round_trips_but_drops_a_reason_of_two_words() {
        let line = StatusLine::new(Version::V1_1, 200, "OK");
        assert_eq!(line.to_string(), "HTTP/1.1 200 OK\r\n");
        assert_eq!(StatusLine::parse("HTTP/1.1 200 OK\r\n"), Some(line));

        // `strVer.size() == 3` is the only case the C++ takes the reason for
        let not_found = StatusLine::parse("HTTP/1.1 404 Not Found\r\n").unwrap();
        assert_eq!(not_found.status_code, 404);
        assert_eq!(not_found.reason_phrase, "");

        // and a code that is not one is a 0
        let none = StatusLine::parse("HTTP/1.1 abc\r\n").unwrap();
        assert_eq!(none.status_code, 0);
        assert_eq!(StatusLine::parse("HTTP/9.9 200 OK\r\n"), None);
        assert_eq!(StatusLine::parse("HTTP/1.1\r\n"), None);
    }

    #[test]
    fn a_field_is_found_whichever_way_its_name_was_written() {
        let mut fields = HeaderFields::new();
        fields.set("Content-Length", "5");
        fields.set("content-type", "text/plain");

        assert_eq!(fields.get("CONTENT-LENGTH"), Some("5"));
        assert_eq!(fields.get("Content-Type"), Some("text/plain"));
        assert_eq!(fields.get(SERVER), None);
        assert_eq!(fields.len(), 2);
        assert!(!fields.is_empty());

        // and one that is set twice keeps the name of the first time
        fields.set("CONTENT-LENGTH", "7");
        assert_eq!(fields.len(), 2);
        assert_eq!(fields.headers()[0].name, "Content-Length");
        assert_eq!(fields.get(CONTENT_LENGTH), Some("7"));
    }

    #[test]
    fn a_field_of_no_value_is_taken_away() {
        let mut fields = HeaderFields::new();
        fields.set(CONNECTION, CLOSE);
        fields.manipulate(USER_AGENT, "");
        assert_eq!(fields.len(), 1, "an empty value is not a field");

        fields.manipulate(CONNECTION, "  ");
        assert!(fields.is_empty(), "and neither is one of spaces");

        fields.manipulate(CONNECTION, KEEPALIVE);
        assert_eq!(fields.get(CONNECTION), Some(KEEPALIVE));
    }

    #[test]
    fn the_head_is_written_in_the_order_it_was_set() {
        let mut fields = HeaderFields::new();
        fields.set_accept_all();
        fields.set_connection_close();
        assert_eq!(fields.to_string(), "Accept: */*\r\nConnection: close\r\n");
        assert!(fields.is_connection_close());
        assert!(!fields.is_connection_keep_alive());
        assert!(!fields.is_chunked());

        fields.set_chunked();
        assert!(fields.is_chunked());
        // whatever way it was written
        fields.set(TRANSFER_ENCODING, "CHUNKED");
        assert!(fields.is_chunked());
    }

    #[test]
    fn the_short_link_settings_are_the_fields_they_say() {
        let mut fields = HeaderFields::new();
        fields.set_content_length(1024);
        fields.set_cache_control_no_cache();
        fields.set_content_type_octet_stream();
        fields.set_user_agent_micro_message();
        fields.set_accept_encoding_gzip();
        fields.set_accept_encoding_deflate();
        fields.set_connection_keepalive();

        assert_eq!(fields.content_length(), 1024);
        assert!(fields.is_connection_keep_alive());
        assert_eq!(fields.get(CACHE_CONTROL), Some(NO_CACHE));
        assert_eq!(fields.get(CONTENT_TYPE), Some(OCTET_STREAM));
        assert_eq!(fields.get(USER_AGENT), Some(USER_AGENT_MICRO_MESSAGE));
        // the two encodings are the same field
        assert_eq!(fields.get(ACCEPT_ENCODING), Some(DEFLATE));
        assert_eq!(HeaderFields::new().content_length(), 0, "no such field");
    }

    #[test]
    fn a_keep_alive_timeout_is_five_seconds_unless_the_peer_said_otherwise() {
        let mut fields = HeaderFields::new();
        fields.set(KEEP_ALIVE, "timeout=10");
        assert_eq!(
            fields.keep_alive_timeout(),
            DEFAULT_KEEP_ALIVE_TIMEOUT,
            "no `Connection` at all"
        );

        fields.set_connection_keepalive();
        assert_eq!(fields.keep_alive_timeout(), 10);
        assert_eq!(DEFAULT_KEEP_ALIVE_TIMEOUT, 5);

        // `timeout > 0 && timeout < 60`, and the C++ gives up on the whole
        // header as soon as it has read one
        fields.set(KEEP_ALIVE, "timeout=0, timeout=7");
        assert_eq!(fields.keep_alive_timeout(), DEFAULT_KEEP_ALIVE_TIMEOUT);
        fields.set(KEEP_ALIVE, "timeout=60");
        assert_eq!(fields.keep_alive_timeout(), DEFAULT_KEEP_ALIVE_TIMEOUT);
        fields.set(KEEP_ALIVE, "max=100");
        assert_eq!(fields.keep_alive_timeout(), DEFAULT_KEEP_ALIVE_TIMEOUT);
        fields.set(KEEP_ALIVE, "");
        assert_eq!(fields.keep_alive_timeout(), DEFAULT_KEEP_ALIVE_TIMEOUT);
    }

    #[test]
    fn a_range_says_which_bytes_and_a_content_range_says_how_many() {
        let mut fields = HeaderFields::new();
        assert_eq!(fields.range(), None);

        fields.set(RANGE, "bytes=0-1024");
        assert_eq!(fields.range(), Some((0, 1024)));
        fields.set(RANGE, "bytes=-100");
        assert_eq!(fields.range(), Some((0, 100)));
        fields.set(RANGE, "0-1024");
        assert_eq!(fields.range(), None, "not `bytes=`");

        let content_range = ContentRange {
            start: 0,
            end: 102_400,
            total: 102_399,
        };
        assert_eq!(
            HeaderFields::content_range_of("bytes 0-102400/102399"),
            Some(content_range)
        );
        fields.set(CONTENT_RANGE, "bytes 0-102400/102399");
        assert_eq!(fields.content_range(), Some(content_range));
        assert_eq!(
            HeaderFields::content_range_of("bytes 0-102400"),
            None,
            "no total"
        );
        fields.set(CONTENT_RANGE, "0-102400/102399");
        assert_eq!(fields.content_range(), None, "not `bytes `");
    }

    #[test]
    fn a_head_is_parsed_out_of_a_block() {
        let mut fields = HeaderFields::new();
        fields.parse("Host: short.example\r\nContent-Length:  5 \r\n\r\n");
        assert_eq!(fields.get(HOST), Some("short.example"));
        assert_eq!(fields.content_length(), 5, "trimmed");

        // a line with no colon in it is the C++'s own arithmetic
        let mut odd = HeaderFields::new();
        odd.parse("nonsense\r\n:\r\nempty:\r\n\r\n");
        assert_eq!(odd.get("nonsense"), Some("nonsense"));
        assert_eq!(odd.len(), 1, "a line of colons and one with no value");
    }

    #[test]
    fn a_request_is_its_first_line_its_head_and_its_body() {
        let mut builder = Builder::new(CsMode::Request);
        builder.request_mut().method = Method::Post;
        builder.request_mut().url = "/cgi".to_string();
        builder.request_mut().version = Version::V1_1;
        builder.fields_mut().set_connection_close();
        builder.set_body(Body::Block(b"hello".to_vec()));

        let request = builder.to_buffer().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&request),
            "POST /cgi HTTP/1.1\r\nConnection: close\r\nContent-Length: 5\r\n\r\nhello"
        );
        // `FillData` moved it out, and the body is gone
        assert_eq!(builder.body(), None);
    }

    #[test]
    fn a_head_on_its_own_is_what_a_request_of_no_body_is() {
        let mut builder = Builder::new(CsMode::Respond);
        builder.status_mut().status_code = 200;
        builder.status_mut().reason_phrase = "OK".to_string();
        builder.status_mut().version = Version::V1_1;
        builder.fields_mut().set_content_length(0);

        assert_eq!(
            String::from_utf8_lossy(&builder.to_buffer().unwrap()),
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"
        );
        // ... and a head with no fields in it is not written at all
        let mut bare = Builder::new(CsMode::Request);
        assert_eq!(bare.header_to_buffer(), None);
        assert_eq!(bare.to_buffer(), None);
    }

    #[test]
    fn a_body_of_no_bytes_writes_nothing_at_all() {
        let mut builder = Builder::new(CsMode::Request);
        builder.fields_mut().set_connection_close();
        builder.set_body(Body::Block(Vec::new()));
        assert_eq!(builder.to_buffer(), Some(Vec::new()));
    }

    #[test]
    fn a_stream_body_is_written_as_chunks() {
        let mut chunks = chunk_header(5).into_bytes();
        chunks.extend_from_slice(b"hello");
        chunks.extend_from_slice(CHUNK_TAIL.as_bytes());
        chunks.extend_from_slice(CHUNK_EOF.as_bytes());
        assert_eq!(chunk_header(5), "5\r\n");
        assert_eq!(CHUNK_EOF, "0\r\n\r\n");

        let mut builder = Builder::new(CsMode::Request);
        builder.fields_mut().set_connection_close();
        builder.set_body(Body::Chunks(chunks.clone()));

        let request = builder.to_buffer().unwrap();
        assert!(String::from_utf8_lossy(&request).contains("Transfer-Encoding: chunked\r\n"));
        assert!(request.ends_with(&chunks));
    }

    #[test]
    fn an_answer_that_came_in_whole_is_read_in_one_go() {
        let mut parser = Parser::new();
        assert_eq!(parser.recv_status(), RecvStatus::Start);
        assert_eq!(parser.mode(), CsMode::Respond);

        let status = parser.recv(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello");
        assert_eq!(status, RecvStatus::End);
        assert!(parser.is_success());
        assert!(parser.is_first_line_ready());
        assert!(parser.is_fields_ready());
        assert!(parser.is_body_ready());
        assert_eq!(parser.status().status_code, 200);
        assert_eq!(parser.status().reason_phrase, "OK");
        assert_eq!(parser.fields().content_length(), 5);
        assert_eq!(parser.body(), b"hello");
        assert_eq!(parser.first_line_len(), 17);
        assert_eq!(parser.header_len(), 21);
        assert!(parser.buffered().is_empty());
    }

    #[test]
    fn an_answer_that_came_in_pieces_is_read_as_they_come() {
        let mut parser = Parser::new();
        assert_eq!(
            parser.recv(b"HTTP/1.1 200 OK\r\nContent-Len"),
            RecvStatus::HeaderFields
        );
        assert!(parser.is_first_line_ready());
        assert_eq!(parser.status().status_code, 200);

        assert_eq!(parser.recv(b"gth: 5\r\n\r\nhe"), RecvStatus::Body);
        assert_eq!(parser.fields().content_length(), 5);
        assert!(parser.is_body_recving());
        assert_eq!(parser.body_len(), 2);

        assert_eq!(parser.recv(b"llo"), RecvStatus::End);
        assert_eq!(parser.body(), b"hello");
    }

    #[test]
    fn a_first_line_that_is_the_whole_head_has_no_fields() {
        let mut parser = Parser::new();
        // a head of no fields is a body of no bytes, which is a whole answer
        assert_eq!(
            parser.recv(b"HTTP/1.1 401 Unauthorized\r\n\r\n"),
            RecvStatus::End
        );
        assert_eq!(parser.status().status_code, 401);
        assert_eq!(parser.fields().len(), 0);
        assert!(parser.is_fields_ready());
    }

    #[test]
    fn a_request_that_came_in_is_read_the_same_way() {
        let mut parser = Parser::new();
        assert_eq!(
            parser.recv(b"POST /cgi HTTP/1.1\r\nHost: h\r\nContent-Length: 2\r\n\r\nhi"),
            RecvStatus::End
        );
        assert_eq!(parser.mode(), CsMode::Request);
        assert_eq!(parser.request().method, Method::Post);
        assert_eq!(parser.request().url, "/cgi");
        assert_eq!(parser.fields().get(HOST), Some("h"));
    }

    #[test]
    fn an_answer_of_chunks_is_read_chunk_by_chunk() {
        let mut parser = Parser::new();
        let status = parser.recv(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n",
        );
        assert_eq!(status, RecvStatus::End);
        assert_eq!(parser.body(), b"hello world");

        // and the same one in pieces
        let mut parser = Parser::new();
        assert_eq!(
            parser.recv(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhel"),
            RecvStatus::Body
        );
        assert!(parser.body().is_empty(), "the chunk is not whole yet");
        assert_eq!(parser.recv(b"lo\r\n"), RecvStatus::Body);
        assert_eq!(parser.body(), b"hello");
        assert_eq!(parser.recv(b"0\r\n\r\n"), RecvStatus::End);
    }

    #[test]
    fn a_body_that_is_not_the_one_the_head_said_is_an_error() {
        let mut parser = Parser::new();
        let status =
            parser.recv(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhelloXX");
        assert_eq!(status, RecvStatus::BodyError);
        assert!(parser.is_error());

        // a chunk bigger than 4g
        let mut parser = Parser::new();
        assert_eq!(
            parser.recv(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffff\r\n"),
            RecvStatus::BodyError
        );

        // and a `Content-Length` of more than 4g — 4g itself is still one the
        // C++ takes, which is its `> kMaxContentLength`
        let mut parser = Parser::new();
        assert_eq!(
            parser.recv(b"HTTP/1.1 200 OK\r\nContent-Length: 4294967297\r\n\r\n"),
            RecvStatus::BodyError
        );
    }

    #[test]
    fn a_first_line_or_a_head_that_never_ends_is_an_error() {
        let mut parser = Parser::new();
        assert_eq!(parser.recv(b"nonsense\r\n"), RecvStatus::FirstLineError);
        assert!(parser.is_error());

        let mut long = Parser::new();
        let line = "X".repeat(MAX_FIRST_LINE + 1);
        assert_eq!(long.recv(line.as_bytes()), RecvStatus::FirstLineError);

        let mut head = Parser::new();
        head.recv(b"HTTP/1.1 200 OK\r\n");
        let fields = "X".repeat(MAX_HEADER_FIELDS + 1);
        assert_eq!(head.recv(fields.as_bytes()), RecvStatus::HeaderFieldsError);
    }

    #[test]
    fn a_link_the_peer_hung_up_on_ends_a_body_of_no_length() {
        let mut parser = Parser::new();
        assert_eq!(
            parser.recv(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nhello"),
            RecvStatus::Body
        );
        assert_eq!(parser.body(), b"hello");
        // a body with no `Content-Length` is however much came before the close
        assert_eq!(parser.recv(b""), RecvStatus::End);
    }

    #[test]
    fn a_read_of_nothing_says_nothing_about_an_answer_that_keeps_going() {
        let mut parser = Parser::new();
        assert_eq!(parser.recv(b""), RecvStatus::Start, "nothing came in yet");
        assert_eq!(
            parser.recv(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhe"),
            RecvStatus::Body
        );
        assert_eq!(
            parser.recv(b""),
            RecvStatus::Body,
            "not a `Connection: close`"
        );
        assert_eq!(parser.recv(b"llo"), RecvStatus::End);
    }

    #[test]
    fn a_head_can_be_read_without_the_body_that_follows_it() {
        let mut parser = Parser::new();
        let status = parser.recv_header_only(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello");
        assert_eq!(status, RecvStatus::Body);
        assert_eq!(parser.fields().content_length(), 5);
        assert!(parser.body().is_empty(), "the body was not read");
        assert_eq!(parser.buffered(), b"hello");
        assert_eq!(parser.recv_header_only(b""), RecvStatus::Body);
    }

    #[test]
    fn a_head_a_caller_changed_is_the_one_the_body_is_read_by() {
        let mut parser = Parser::new();
        parser.recv_header_only(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello");
        parser.fields_mut().set_content_length(0);

        // a body of no bytes is a whole answer
        assert_eq!(parser.recv(b""), RecvStatus::Body);
        assert!(parser.body().is_empty());
    }
}
