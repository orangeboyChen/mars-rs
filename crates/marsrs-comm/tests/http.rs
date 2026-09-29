//! What the C++'s http answers for the same calls, through the public api.
//!
//! The samples are what `mars/stn/proto/shortlink_packer.cc` writes for one
//! short-link request — a `POST` of five bytes on a link that is closed when
//! the answer came back — and what `mars/stn/src/shortlink.cc` reads: a 200
//! with a `Content-Length`, one that comes in two pieces, one the peer hung up
//! on with no length at all, an answer of chunks, and a status that is not 200.

use marsrs_comm::http::{
    Body, Builder, CsMode, Method, Parser, RecvStatus, RequestLine, StatusLine, Version,
};

/// `shortlink_pack` — the fields the C++ sets, which is not the order they
/// come out in: its `HeaderFields` is a map ordered by name, and neither name's
/// case is in the way.
fn a_request() -> Builder {
    let mut builder = Builder::new(CsMode::Request);
    *builder.request_mut() =
        RequestLine::new(Method::Post, "/cgi-bin/micromsg-bin/short", Version::V1_1);
    let fields = builder.fields_mut();
    fields.set_accept_all();
    fields.set_user_agent_micro_message();
    fields.set_cache_control_no_cache();
    fields.set_content_type_octet_stream();
    fields.set_connection_close();
    builder
}

const HEAD: &str = "POST /cgi-bin/micromsg-bin/short HTTP/1.1\r\n\
                    Accept: */*\r\n\
                    Cache-Control: no-cache\r\n\
                    Connection: close\r\n\
                    Content-Type: application/octet-stream\r\n\
                    User-Agent: MicroMessenger Client\r\n";

/// The same head as [`HEAD`] with the `Content-Length` a body of five bytes is
/// given, which sorts in between `Connection` and `Content-Type` and not at the
/// end of the head.
const HEAD_OF_FIVE_BYTES: &str = "POST /cgi-bin/micromsg-bin/short HTTP/1.1\r\n\
                    Accept: */*\r\n\
                    Cache-Control: no-cache\r\n\
                    Connection: close\r\n\
                    Content-Length: 5\r\n\
                    Content-Type: application/octet-stream\r\n\
                    User-Agent: MicroMessenger Client\r\n";

#[test]
fn a_short_link_request_is_its_head_and_its_body() {
    let mut builder = a_request();
    builder.set_body(Body::Block(b"hello".to_vec()));

    let request = builder.to_buffer().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&request),
        format!("{HEAD_OF_FIVE_BYTES}\r\nhello")
    );
    // the head on its own, which is what a body of no bytes is
    let head_only = a_request();
    assert_eq!(
        String::from_utf8_lossy(&head_only.header_to_buffer().unwrap()),
        format!("{HEAD}\r\n")
    );
}

#[test]
fn an_answer_of_two_hundred_is_read_off_the_wire() {
    let mut parser = Parser::new();
    assert_eq!(
        parser.recv(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello"),
        RecvStatus::End
    );
    assert_eq!(parser.mode(), CsMode::Respond);
    assert_eq!(
        parser.status(),
        &StatusLine::new(Version::V1_1, 200, "OK"),
        "what the short link asks before it answers the task"
    );
    assert_eq!(parser.body(), b"hello");
    assert_eq!(parser.fields().len(), 1);
    assert!(parser.is_success());
}

#[test]
fn an_answer_that_came_in_two_pieces_is_whole_when_the_last_one_did() {
    let mut parser = Parser::new();
    // the head ended in the middle of a field, and the body came after it
    assert_eq!(
        parser.recv(b"HTTP/1.1 200 OK\r\nContent-Le"),
        RecvStatus::HeaderFields
    );
    assert!(parser.is_first_line_ready());
    assert_eq!(parser.recv(b"ngth: 5\r\n\r\nhel"), RecvStatus::Body);
    assert!(parser.is_body_recving());
    assert!(!parser.is_success());
    assert_eq!(parser.recv(b"lo"), RecvStatus::End);
    assert_eq!(parser.body(), b"hello");
}

#[test]
fn an_answer_the_peer_hung_up_on_is_however_much_came_before() {
    // a request of `Connection: close` is answered with no `Content-Length`,
    // which is a body that ends when the socket does
    let mut parser = Parser::new();
    assert_eq!(
        parser.recv(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nhello"),
        RecvStatus::Body
    );
    assert_eq!(parser.body(), b"hello");
    assert_eq!(parser.recv(b""), RecvStatus::End, "the peer hung up");
    assert!(parser.is_success());
}

#[test]
fn an_answer_of_chunks_is_one_body() {
    let mut parser = Parser::new();
    assert_eq!(
        parser.recv(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"
        ),
        RecvStatus::End
    );
    assert!(parser.fields().is_chunked());
    assert_eq!(parser.body(), b"hello world");
}

#[test]
fn a_status_that_is_not_two_hundred_is_still_an_answer() {
    let mut parser = Parser::new();
    assert_eq!(
        parser.recv(b"HTTP/1.1 500 Server Error\r\nContent-Length: 0\r\n\r\n"),
        RecvStatus::End
    );
    assert_eq!(parser.status().status_code, 500);
    assert_eq!(
        parser.status().reason_phrase,
        "",
        "`strVer.size() == 3` is the only case the C++ takes the reason for"
    );
    assert!(parser.body().is_empty());
}

#[test]
fn a_head_that_is_read_on_its_own_leaves_the_body_behind() {
    let mut parser = Parser::new();
    assert_eq!(
        parser.recv_header_only(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello"),
        RecvStatus::Body
    );
    assert!(parser.is_fields_ready());
    assert_eq!(parser.fields().content_length(), 5);
    assert!(parser.body().is_empty());
    assert_eq!(parser.buffered(), b"hello", "what is left to read");
}

#[test]
fn a_number_that_does_not_fit_is_the_longest_body_there_can_be() {
    // `strtoull` saturates at `ULLONG_MAX`, so a length nobody can deliver is
    // a body that cannot be read — where a parse that fails on overflow would
    // have answered `0`, and an empty body that is done
    let mut parser = Parser::new();
    assert_eq!(
        parser.recv(b"HTTP/1.1 200 OK\r\nContent-Length: 99999999999999999999999\r\n\r\n"),
        RecvStatus::BodyError
    );
    assert_eq!(parser.fields().content_length(), u64::MAX);

    // ... and the same for a chunk: a size that big is not the last chunk
    let mut parser = Parser::new();
    assert_eq!(
        parser.recv(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffffff\r\n"),
        RecvStatus::BodyError
    );

    // `-1` is what `strtoull` makes of a negative number: it wraps around
    let mut parser = Parser::new();
    assert_eq!(
        parser.recv(b"HTTP/1.1 200 OK\r\nContent-Length: -1\r\n\r\n"),
        RecvStatus::BodyError
    );
}

/// The last chunk is a size of nothing, then the trailer section, then an
/// empty line — `0\r\nTrailer-Field: v\r\n\r\n`. Looking for a single
/// `CRLF` in the trailer took the end of the first field for the end of
/// the whole body and left the empty line in the buffer.
#[test]
fn a_chunked_answer_with_trailer_fields_ends_at_the_empty_line() {
    let mut parser = Parser::new();
    assert_eq!(
        parser.recv(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\nTrailer-Field: v\r\n\r\n"
        ),
        RecvStatus::End
    );
    assert_eq!(parser.body(), b"hello");
    assert_eq!(
        parser.buffered(),
        b"",
        "the empty line that ends the trailer was left in the buffer"
    );
}

/// What that stray `CRLF` did to the answer that came next on the same
/// connection: handed to a parser — which is what a keep-alive connection
/// does with the bytes that are left — it is not a first line at all.
#[test]
fn the_answer_after_a_chunked_one_is_read_off_the_same_connection() {
    let mut parser = Parser::new();
    assert_eq!(
        parser.recv(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\nTrailer-Field: v\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi"
        ),
        RecvStatus::End
    );
    assert_eq!(parser.body(), b"hello");

    let mut next = Parser::new();
    assert_eq!(next.recv(parser.buffered()), RecvStatus::End);
    assert_eq!(next.body(), b"hi");
}

/// `MAX_CHUNK_LENGTH` is 4g, and a chunk that big is one this parser does
/// not read: on a 32-bit target — `armv7` is one this repository builds —
/// `size as usize` truncated it to 0, and `begin + size` either wrapped or
/// read the chunk's own bytes as the `CRLF` that ends it.
#[test]
fn a_chunk_of_the_biggest_size_there_is_is_not_read_as_one_of_nothing() {
    // `100000000` is the limit itself, and `ffffffff` one byte below it
    for size in ["100000000", "ffffffff"] {
        let mut parser = Parser::new();
        let status = parser.recv(
            format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{size}\r\nabc")
                .as_bytes(),
        );
        // a body that is not read, and no overflow on the way there: which
        // of the two it is depends on how wide a `usize` is
        assert!(!parser.is_success(), "{size} of chunk: {status:?}");
        assert!(parser.body().is_empty(), "{size} of chunk was read as none");
    }
}

#[test]
fn the_size_of_a_chunk_is_hexadecimal() {
    // `strtoull(_, 16)` reads the `0x` in front of it, which a parse of the
    // digits alone does not: `0` is the chunk that ends the body
    let mut parser = Parser::new();
    assert_eq!(
        parser.recv(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0x2\r\nhi\r\n0\r\n\r\n"),
        RecvStatus::End
    );
    assert_eq!(parser.body(), b"hi");
}
