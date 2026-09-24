//! What a short-link task goes out as, driven the way
//! `mars/stn/src/shortlink.cc` drives it: the url and the head the run works
//! out of its connect, the [`pack`] that writes them, and the answer that comes
//! back.
//!
//! The samples are the ones the C++ would write for a task on
//! `/cgi-bin/micromsg-bin/short` — one on its own host, one through an http
//! proxy it has to log in to, and one of its own headers — and the answers it
//! reads: a 200 with a body, a 302, and one that keeps the socket.

use std::collections::BTreeMap;

use mars_comm::http::{Parser, RecvStatus};
use mars_comm::{ProxyInfo, ProxyType};
use mars_stn::shortlink::{answer, Answer};
use mars_stn::{
    default_packer, is_keep_alive, keep_alive, pack, request_headers, request_url, ConnectProfile,
    IpPortItem, IpSourceType, KeepAlive, Task,
};

/// A task of five bytes on the short-link host, and the connect that was made
/// for it.
fn a_task() -> (Task, ConnectProfile) {
    let mut task = Task::new(7, 12);
    task.cgi = "/cgi-bin/micromsg-bin/short".to_string();
    task.shortlink_host_list = vec!["short.weixin.qq.com".to_string()];

    let mut profile = ConnectProfile::new();
    profile.host = "short.weixin.qq.com".to_string();
    profile.ip = "183.3.226.35".to_string();
    profile.port = 80;
    profile.ip_type = IpSourceType::Dns;
    profile.ip_items = vec![IpPortItem::new("183.3.226.35", 80)];
    (task, profile)
}

/// What the run writes: the url of the connect, the head of the task, and then
/// the body.
fn request_of(task: &Task, profile: &ConnectProfile, body: &[u8]) -> Vec<u8> {
    let url = request_url(profile, &task.cgi);
    let headers = request_headers(profile, task);
    pack(&url, &headers, body)
}

/// The five fields mars writes for itself, in its order, and the empty line
/// that ends the head.
const FIELDS: &str = "Accept: */*\r\n\
                      User-Agent: MicroMessenger Client\r\n\
                      Cache-Control: no-cache\r\n\
                      Content-Type: application/octet-stream\r\n\
                      Connection: close\r\n";

#[test]
fn a_short_link_task_goes_out_as_a_post_of_its_cgi() {
    let (task, profile) = a_task();
    let request = request_of(&task, &profile, b"hello");

    assert_eq!(
        String::from_utf8_lossy(&request),
        format!(
            "POST /cgi-bin/micromsg-bin/short HTTP/1.1\r\n\
             {FIELDS}\
             Content-Length: 5\r\n\
             Host: short.weixin.qq.com\r\n\
             \r\n\
             hello"
        )
    );
}

#[test]
fn a_task_of_no_body_goes_out_as_a_head_that_says_so() {
    let (task, profile) = a_task();
    let request = request_of(&task, &profile, b"");

    assert_eq!(
        String::from_utf8_lossy(&request),
        format!(
            "POST /cgi-bin/micromsg-bin/short HTTP/1.1\r\n\
             {FIELDS}\
             Content-Length: 0\r\n\
             Host: short.weixin.qq.com\r\n\
             \r\n"
        ),
        "the C++ writes the head and then whatever the body is, which is nothing"
    );
}

#[test]
fn a_task_that_goes_through_a_proxy_asks_for_the_whole_url() {
    let (task, mut profile) = a_task();
    profile.ip_type = IpSourceType::Proxy;
    profile.proxy_info = ProxyInfo::new(ProxyType::Http, "", "10.0.0.1", 8080, "mars", "secret");

    let request = request_of(&task, &profile, b"hello");
    assert_eq!(
        String::from_utf8_lossy(&request),
        format!(
            "POST http://short.weixin.qq.com/cgi-bin/micromsg-bin/short HTTP/1.1\r\n\
             {FIELDS}\
             Content-Length: 5\r\n\
             Host: short.weixin.qq.com\r\n\
             Proxy-Authorization: Basic bWFyczpzZWNyZXQ=\r\n\
             \r\n\
             hello"
        ),
        "a proxy is asked for the whole url, and logged in to"
    );
}

#[test]
fn a_task_of_its_own_headers_writes_them_and_asks_to_be_kept() {
    let (task, profile) = a_task();
    let mut task = task;
    task.headers
        .insert("Connection".to_string(), "Keep-Alive".to_string());
    task.headers
        .insert("X-Online-Host".to_string(), "other".to_string());

    assert!(
        is_keep_alive(&task),
        "what decides whether the socket is kept"
    );

    let request = String::from_utf8_lossy(&request_of(&task, &profile, b"hello")).to_string();
    assert!(
        request.contains("Connection: Keep-Alive\r\n"),
        "its own, not the one mars writes: {request}"
    );
    assert!(request.contains("X-Online-Host: other\r\n"));
    assert!(
        !is_keep_alive(&{
            let mut lowered = task.clone();
            lowered
                .headers
                .insert("Connection".to_string(), "keep-alive".to_string());
            lowered
        }),
        "the C++ compares the value byte for byte"
    );
}

#[test]
fn a_head_of_the_task_s_own_is_sorted_by_name() {
    let (task, profile) = a_task();
    let mut task = task;
    task.headers
        .insert("X-Zebra".to_string(), "last".to_string());
    task.headers
        .insert("X-Alpha".to_string(), "first".to_string());

    let headers: BTreeMap<String, String> = request_headers(&profile, &task);
    let names: Vec<&String> = headers.keys().collect();
    assert_eq!(names, ["Host", "X-Alpha", "X-Zebra"]);
}

#[test]
fn an_answer_of_two_hundred_is_the_body_and_any_other_is_the_status() {
    let mut parser = Parser::new();
    assert_eq!(
        parser.recv(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello"),
        RecvStatus::End
    );
    assert_eq!(answer(&parser), Some(Answer::Ok(b"hello".as_slice())));
    assert_eq!(answer(&parser).unwrap().status(), 200);

    // a redirect is an answer the C++ does not answer the task with
    let mut redirected = Parser::new();
    assert_eq!(
        redirected.recv(b"HTTP/1.1 302 Found\r\nLocation: /other\r\nContent-Length: 0\r\n\r\n"),
        RecvStatus::End
    );
    let answer = answer(&redirected).unwrap();
    assert_eq!(answer, Answer::Http(302));
    assert_eq!(answer.status(), 302);
    assert_eq!(mars_stn::ErrCmdType::Http, answer.err_cmd_type());
}

#[test]
fn an_answer_that_came_in_two_pieces_is_answered_when_the_last_one_did() {
    let mut parser = Parser::new();
    parser.recv(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nhello");
    assert_eq!(
        answer(&parser),
        None,
        "the C++ only asks the status once the parser is at the end"
    );
    assert_eq!(parser.recv(b" world"), RecvStatus::End);
    assert_eq!(answer(&parser), Some(Answer::Ok(b"hello world".as_slice())));
}

#[test]
fn a_socket_the_server_kept_is_the_one_the_next_task_reuses() {
    let mut parser = Parser::new();
    parser.recv(b"HTTP/1.1 200 OK\r\nConnection: Keep-Alive\r\nKeep-Alive: timeout=15\r\n\r\n");

    assert_eq!(
        keep_alive(parser.fields(), true, Task::TRANSPORT_PROTOCOL_TCP),
        KeepAlive::Reuse { timeout: 15 },
        "for as long as the server said"
    );
    assert_eq!(
        keep_alive(parser.fields(), true, Task::TRANSPORT_PROTOCOL_QUIC),
        KeepAlive::Reuse { timeout: 30 },
        "thirty seconds on QUIC, whatever the server said"
    );
    assert_eq!(
        keep_alive(parser.fields(), false, Task::TRANSPORT_PROTOCOL_TCP),
        KeepAlive::Closed,
        "a request that did not ask is closed when it is answered"
    );

    let mut closed = Parser::new();
    closed.recv(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
    assert_eq!(
        keep_alive(closed.fields(), true, Task::TRANSPORT_PROTOCOL_TCP),
        KeepAlive::Closed
    );

    // a server that said nothing about it is kept for the default
    let mut silent = Parser::new();
    silent.recv(b"HTTP/1.1 200 OK\r\nConnection: Keep-Alive\r\nContent-Length: 0\r\n\r\n");
    assert_eq!(
        keep_alive(silent.fields(), true, Task::TRANSPORT_PROTOCOL_TCP),
        KeepAlive::Reuse { timeout: 5 }
    );
}

#[test]
fn the_packer_the_app_replaced_is_the_one_that_writes() {
    let (task, profile) = a_task();
    let headers = request_headers(&profile, &task);

    assert_eq!(
        default_packer()("/cgi", &headers, b"hello"),
        pack("/cgi", &headers, b"hello")
    );

    // one that writes its own head, which is what the app hands a link instead
    let own: Box<mars_stn::Packer> =
        Box::new(|url, _, body| format!("GET {url}\r\n\r\n{}", body.len()).into_bytes());
    let (task, profile) = a_task();
    let headers = request_headers(&profile, &task);
    assert_eq!(
        own("/cgi-bin/micromsg-bin/short", &headers, b"hello"),
        b"GET /cgi-bin/micromsg-bin/short\r\n\r\n5".to_vec()
    );
}
