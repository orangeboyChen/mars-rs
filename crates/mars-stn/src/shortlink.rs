//! `mars/stn/proto/shortlink_packer.cc` — what a short-link task goes out as,
//! and what comes back for it.
//!
//! A short link is one http request on its own socket: [`pack`] is the whole
//! of the C++'s `shortlink_pack` — a `POST` of the cgi, five fields of its own,
//! a `Content-Length`, the headers [`request_headers`] worked out, and then the
//! body — and [`Answer`] is the fork the C++ makes of what came back: a status
//! of 200 is the body, anything else is `kEctHttp` with the status.
//!
//! The url and the headers are what `mars/stn/src/shortlink.cc` hands the
//! packer, and they are pure functions of the task and of the connect that was
//! made for it, which is why they are here: a socket is not needed to know what
//! would have been written on one.
//!
//! Two things of the C++ are not ported. The `X-Online-Host` field, which only
//! the `_WIN32` branch writes. And the `AutoBuffer _extension` the C++ hands
//! the packer: the packer mars ships never looks at it, so neither does this
//! one.

use std::collections::BTreeMap;

use mars_comm::base64;
use mars_comm::http::{
    Builder, CsMode, HeaderFields, Method, Parser, RecvStatus, RequestLine, Version, CONNECTION,
    HOST, KEEPALIVE, PROXY_AUTHORIZATION,
};
use mars_comm::{ProxyInfo, ProxyType};

use crate::{ConnectProfile, ErrCmdType, IpSourceType, Task};

/// `std::map<std::string, std::string>` — the headers of a short-link task.
///
/// The C++'s is sorted, so a [`BTreeMap`] is one: the fields go out in the
/// order of their names, whichever order the caller put them in.
pub type Headers = BTreeMap<String, String>;

/// `shortlink_pack` — the app replaces this in the C++ (the symbol is a weak
/// one), so a link here holds one instead: [`default_packer`] is what it holds
/// unless it is handed another.
pub type Packer = dyn Fn(&str, &Headers, &[u8]) -> Vec<u8> + Send;

/// `shortlink_pack` as a value — the packer mars ships, boxed so a link can
/// hold it.
pub fn default_packer() -> Box<Packer> {
    Box::new(pack)
}

/// `shortlink_pack` — the request, head first and then the body.
///
/// The five fields are the C++'s own, in its order; `Content-Length` is the
/// length of the body, and the headers after it are the ones the caller asked
/// for, which is why a `Content-Length` or a `Connection` of theirs is the one
/// that goes out.
///
/// A body of no bytes is a head with `Content-Length: 0` — the C++ writes the
/// head with `HeaderToBuffer` and then whatever the body is, which is nothing.
pub fn pack(url: &str, headers: &Headers, body: &[u8]) -> Vec<u8> {
    let mut builder = Builder::new(CsMode::Request);
    *builder.request_mut() = RequestLine::new(Method::Post, url, Version::V1_1);

    let fields = builder.fields_mut();
    fields.set_accept_all();
    fields.set_user_agent_micro_message();
    fields.set_cache_control_no_cache();
    fields.set_content_type_octet_stream();
    fields.set_connection_close();
    fields.set_content_length(body.len() as u64);

    for (name, value) in headers {
        fields.set(name, value);
    }

    let mut request = builder.header_to_buffer().unwrap_or_default();
    request.extend_from_slice(body);
    request
}

/// `CheckKeepAlive` — whether this task asked for the socket to be kept, which
/// is a `Connection: Keep-Alive` of its own.
///
/// The C++ compares the value byte for byte, so `keep-alive` — which the
/// parser would call keep-alive — is not one.
pub fn is_keep_alive(task: &Task) -> bool {
    task.headers
        .get(CONNECTION)
        .is_some_and(|value| value == KEEPALIVE)
}

/// What `__RunReadWrite` posts to: the cgi, and `http://host` in front of it
/// when the connect went through a proxy — a proxy is asked for the whole url,
/// not for the path.
pub fn request_url(profile: &ConnectProfile, cgi: &str) -> String {
    let mut url = String::new();
    if profile.ip_type == IpSourceType::Proxy {
        url.push_str("http://");
        url.push_str(&profile.host);
    }
    url.push_str(cgi);
    url
}

/// What `__RunReadWrite` puts in the head: the `Host` of the connect, the
/// `Proxy-Authorization` of an http proxy that has an account, and then the
/// task's own headers — which is why one of theirs wins over either.
pub fn request_headers(profile: &ConnectProfile, task: &Task) -> Headers {
    let mut headers = Headers::new();
    headers.insert(HOST.to_string(), profile.host.clone());

    if let Some(authorization) = authorization(&profile.proxy_info) {
        headers.insert(PROXY_AUTHORIZATION.to_string(), authorization);
    }

    for (name, value) in &task.headers {
        headers.insert(name.clone(), value.clone());
    }
    headers
}

/// `Proxy-Authorization: Basic …` — `username:password` in base64, which is
/// what an http proxy is logged in with. `None` is every case the C++ leaves
/// the field out: no proxy, a proxy that is not an http one, or one with no
/// account.
///
/// The C++ writes it through a `snprintf` into 1024 bytes, so a long enough
/// account is cut off there; the port is not cut off.
fn authorization(proxy: &ProxyInfo) -> Option<String> {
    let account = match proxy.kind {
        ProxyType::Http if !proxy.username.is_empty() && !proxy.password.is_empty() => {
            format!("{}:{}", proxy.username, proxy.password)
        }
        _ => return None,
    };
    Some(format!("Basic {}", base64::encode(account.as_bytes())))
}

/// What an answer leaves the socket it came on: the C++ reads the server's
/// `Connection` only when the request asked to be kept, and keeps the socket
/// for the timeout the server gave — or for thirty seconds on QUIC, which is
/// what the C++ hands a QUIC link whatever the server said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepAlive {
    /// The server said close, or the request never asked: the socket is closed
    /// when the answer is done.
    Closed,
    /// The socket goes into the pool, good for this many seconds.
    Reuse {
        /// `keepalive_timeout`.
        timeout: u32,
    },
}

/// The `parse_status == kEnd` branch of the read loop.
pub fn keep_alive(fields: &HeaderFields, requested: bool, transport_protocol: i32) -> KeepAlive {
    if !requested || !fields.is_connection_keep_alive() {
        return KeepAlive::Closed;
    }
    let timeout = if transport_protocol == Task::TRANSPORT_PROTOCOL_QUIC {
        30
    } else {
        fields.keep_alive_timeout()
    };
    KeepAlive::Reuse { timeout }
}

/// What a whole answer is: the C++ asks `status_code != 200` of it and either
/// reports `kEctHttp` with the status or answers the task with the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer<'a> {
    /// `kEctOK` — the body, which is what goes to `Buf2Resp`.
    Ok(&'a [u8]),
    /// `kEctHttp` — a status that is not 200.
    Http(i32),
}

impl Answer<'_> {
    /// The `ErrCmdType` the C++ calls this answer.
    pub fn err_cmd_type(&self) -> ErrCmdType {
        match self {
            Self::Ok(_) => ErrCmdType::Ok,
            Self::Http(_) => ErrCmdType::Http,
        }
    }

    /// The status code of the answer, which is what `disconn_errcode` gets: 200
    /// for one that is good, and the status itself for one that is not.
    pub fn status(&self) -> i32 {
        match self {
            Self::Ok(_) => 200,
            Self::Http(status) => *status,
        }
    }
}

/// `status_code != 200` — the fork, for an answer that is whole. `None` is an
/// answer that is not: the C++ only looks at the status once the parser said
/// `kEnd`.
pub fn answer(parser: &Parser) -> Option<Answer<'_>> {
    if parser.recv_status() != RecvStatus::End {
        return None;
    }
    let status = parser.status().status_code;
    Some(if status == 200 {
        Answer::Ok(parser.body())
    } else {
        Answer::Http(status)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IpPortItem, Task};

    fn task() -> Task {
        let mut task = Task::new(7, 12);
        task.cgi = "/cgi-bin/micromsg-bin/short".to_string();
        task
    }

    fn profile() -> ConnectProfile {
        let mut profile = ConnectProfile::new();
        profile.host = "short.weixin.qq.com".to_string();
        profile.ip = "127.0.0.1".to_string();
        profile.port = 80;
        profile.ip_items = vec![IpPortItem::new("127.0.0.1", 80)];
        profile
    }

    #[test]
    fn a_request_is_a_post_of_the_cgi_and_the_body() {
        let mut headers = Headers::new();
        headers.insert("Host".to_string(), "short.weixin.qq.com".to_string());

        let request = pack("/cgi-bin/micromsg-bin/short", &headers, b"hello");
        assert_eq!(
            String::from_utf8_lossy(&request),
            "POST /cgi-bin/micromsg-bin/short HTTP/1.1\r\n\
             Accept: */*\r\n\
             User-Agent: MicroMessenger Client\r\n\
             Cache-Control: no-cache\r\n\
             Content-Type: application/octet-stream\r\n\
             Connection: close\r\n\
             Content-Length: 5\r\n\
             Host: short.weixin.qq.com\r\n\
             \r\n\
             hello"
        );
    }

    #[test]
    fn a_request_of_no_body_says_how_long_it_is() {
        let request = pack("/cgi-bin/micromsg-bin/short", &Headers::new(), b"");
        assert!(
            request.ends_with("Content-Length: 0\r\n\r\n".as_bytes()),
            "a head that says how long a body it has, and no body"
        );
    }

    #[test]
    fn a_header_of_the_caller_is_the_one_that_goes_out() {
        // `Content-Length` and `Connection` are set before the caller's own
        let mut headers = Headers::new();
        headers.insert("Content-Length".to_string(), "9".to_string());
        headers.insert("Connection".to_string(), "Keep-Alive".to_string());

        let request = pack("/cgi", &headers, b"hello");
        let request = String::from_utf8_lossy(&request);
        assert!(request.contains("Content-Length: 9\r\n"));
        assert!(request.contains("Connection: Keep-Alive\r\n"));
        // and the body is written as it is, whatever the length said
        assert!(request.ends_with("hello"));
    }

    #[test]
    fn a_keep_alive_task_is_one_whose_connection_says_keep_alive() {
        let mut task = task();
        assert!(!is_keep_alive(&task));

        task.headers
            .insert("Connection".to_string(), "Keep-Alive".to_string());
        assert!(is_keep_alive(&task));

        // the C++ compares it byte for byte, so the parser's spelling is not
        // the one it is looking for
        task.headers
            .insert("Connection".to_string(), "keep-alive".to_string());
        assert!(!is_keep_alive(&task));
    }

    #[test]
    fn the_url_is_the_cgi_and_the_whole_thing_through_a_proxy() {
        let profile = profile();
        assert_eq!(request_url(&profile, "/cgi"), "/cgi");

        let mut through_proxy = profile;
        through_proxy.ip_type = IpSourceType::Proxy;
        assert_eq!(
            request_url(&through_proxy, "/cgi"),
            "http://short.weixin.qq.com/cgi"
        );
    }

    #[test]
    fn the_head_is_the_host_and_then_the_task_s_own() {
        let mut task = task();
        task.headers.insert("X-Seq".to_string(), "3".to_string());

        let headers = request_headers(&profile(), &task);
        assert_eq!(
            headers.get("Host"),
            Some(&"short.weixin.qq.com".to_string())
        );
        assert_eq!(headers.get("X-Seq"), Some(&"3".to_string()));
        assert_eq!(headers.len(), 2, "no proxy, so no authorization");

        // a task's own `Host` is the one that goes out
        task.headers
            .insert("Host".to_string(), "other.example".to_string());
        assert_eq!(
            request_headers(&profile(), &task).get("Host"),
            Some(&"other.example".to_string())
        );
    }

    #[test]
    fn an_http_proxy_with_an_account_is_logged_in_with() {
        let mut profile = profile();
        profile.proxy_info =
            ProxyInfo::new(ProxyType::Http, "", "10.0.0.1", 8080, "mars", "secret");

        let headers = request_headers(&profile, &task());
        assert_eq!(
            headers.get("Proxy-Authorization"),
            Some(&"Basic bWFyczpzZWNyZXQ=".to_string())
        );
    }

    #[test]
    fn a_proxy_with_no_account_is_not_logged_in_with() {
        let mut profile = profile();
        // no account at all
        profile.proxy_info = ProxyInfo::new(ProxyType::Http, "", "10.0.0.1", 8080, "", "");
        assert!(!request_headers(&profile, &task()).contains_key("Proxy-Authorization"));

        // a password and no username
        profile.proxy_info = ProxyInfo::new(ProxyType::Http, "", "10.0.0.1", 8080, "", "secret");
        assert!(!request_headers(&profile, &task()).contains_key("Proxy-Authorization"));

        // an account on a proxy that is not an http one
        profile.proxy_info =
            ProxyInfo::new(ProxyType::Socks5, "", "10.0.0.1", 1080, "mars", "secret");
        assert!(!request_headers(&profile, &task()).contains_key("Proxy-Authorization"));
    }

    #[test]
    fn a_socket_is_kept_for_how_long_the_server_said() {
        let mut parser = Parser::new();
        // an answer with no `Content-Length` and no `Connection: close` is
        // over as soon as its head is: an empty body is the whole body
        assert_eq!(
            parser.recv(
                b"HTTP/1.1 200 OK\r\nConnection: Keep-Alive\r\nKeep-Alive: timeout=15\r\n\r\n"
            ),
            RecvStatus::End
        );
        assert_eq!(
            keep_alive(parser.fields(), true, Task::TRANSPORT_PROTOCOL_TCP),
            KeepAlive::Reuse { timeout: 15 },
            "the timeout the server gave"
        );
        assert_eq!(
            keep_alive(parser.fields(), true, Task::TRANSPORT_PROTOCOL_QUIC),
            KeepAlive::Reuse { timeout: 30 },
            "thirty seconds whatever the server said"
        );
        assert_eq!(
            keep_alive(parser.fields(), false, Task::TRANSPORT_PROTOCOL_TCP),
            KeepAlive::Closed,
            "a request that did not ask is not kept"
        );
    }

    #[test]
    fn a_server_that_said_close_is_not_kept() {
        let mut parser = Parser::new();
        parser.recv(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
        assert_eq!(
            keep_alive(parser.fields(), true, Task::TRANSPORT_PROTOCOL_TCP),
            KeepAlive::Closed
        );
    }

    #[test]
    fn an_answer_of_two_hundred_is_the_body_and_any_other_is_the_status() {
        let mut parser = Parser::new();
        // not whole yet, which is when the C++ does not look at the status
        parser.recv(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhel");
        assert_eq!(answer(&parser), None);

        parser.recv(b"lo");
        assert_eq!(answer(&parser), Some(Answer::Ok(b"hello".as_slice())));
        assert_eq!(answer(&parser).unwrap().err_cmd_type(), ErrCmdType::Ok);
        assert_eq!(answer(&parser).unwrap().status(), 200);

        let mut failed = Parser::new();
        failed.recv(b"HTTP/1.1 500 Server Error\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(answer(&failed), Some(Answer::Http(500)));
        assert_eq!(answer(&failed).unwrap().err_cmd_type(), ErrCmdType::Http);
        assert_eq!(answer(&failed).unwrap().status(), 500);
    }

    #[test]
    fn the_packer_a_link_holds_is_the_one_that_writes() {
        let headers = Headers::new();
        assert_eq!(
            default_packer()("/cgi", &headers, b"hello"),
            pack("/cgi", &headers, b"hello")
        );
    }
}
