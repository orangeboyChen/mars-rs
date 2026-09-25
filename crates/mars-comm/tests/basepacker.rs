//! `mars/comm/basepacker.cc` — the wire format the long link spoke before
//! `longlink_packer.cc`.

use mars_comm::basepacker::{
    packer_pack, packer_unpack, simple_int_pack, simple_int_pack_length, simple_int_unpack,
    simple_short_pack, simple_short_pack_length, simple_short_unpack, PackerUnpacked, Refused,
    SimpleUnpacked, HEAD_LEN, LONGLINKPACK_CONTINUE, LONGLINKPACK_CONTINUE_DATA,
    LONGLINKPACK_CONTINUE_HEAD, LONGLINKPACK_FALSE, LONGLINKPACK_OK, MAX_URL_LEN, SIMPLE_CONTINUE,
    SIMPLE_CONTINUE_DATA, SIMPLE_OK, VER,
};

fn head(raw: &[u8]) -> (u8, u8, u8, u8, u32, u32, u32) {
    (
        raw[0],
        raw[1],
        raw[2],
        raw[3],
        u32::from_be_bytes(raw[4..8].try_into().unwrap()),
        u32::from_be_bytes(raw[8..12].try_into().unwrap()),
        u32::from_be_bytes(raw[12..16].try_into().unwrap()),
    )
}

#[test]
fn the_bytes_are_the_ones_the_c_writes() {
    // `Packer_Pack("/cgi-bin/micromsg", 7, "the body", 8, true)`, and the two
    // ends of the hash: the C++ seeds the second call with the first one's
    // answer, which is the same as hashing the URL and the body in one go
    let packed = packer_pack("/cgi-bin/micromsg", 7, b"the body", true);
    assert_eq!(
        packed,
        vec![
            0x4a, 0x01, 0x10, 0x11, 0x00, 0x00, 0x00, 0x29, 0x00, 0x00, 0x00, 0x07, 0x75, 0xdc,
            0x09, 0x67, b'/', b'c', b'g', b'i', b'-', b'b', b'i', b'n', b'/', b'm', b'i', b'c',
            b'r', b'o', b'm', b's', b'g', b't', b'h', b'e', b' ', b'b', b'o', b'd', b'y',
        ]
    );
    assert_eq!(
        mars_comm::adler32::adler32(b"/cgi-bin/micromsgthe body"),
        0x75dc_0967
    );

    // `Packer_Pack("/x", 1, "body", 4, false)` — a hash of `0`, which is what
    // tells `Packer_Unpack` not to check it
    assert_eq!(
        packer_pack("/x", 1, b"body", false),
        vec![
            0x28, 0x01, 0x10, 0x02, 0x00, 0x00, 0x00, 0x16, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
            0x00, 0x00, b'/', b'x', b'b', b'o', b'd', b'y',
        ]
    );
}

#[test]
fn a_package_is_a_header_a_url_and_a_body() {
    let packed = packer_pack("/cgi-bin/micromsg", 7, b"the body", true);

    assert_eq!(packed.len(), HEAD_LEN + 17 + 8);
    let (magic, ver, head_length, url_length, total_length, sequence, hash) = head(&packed);
    assert_eq!(ver, VER);
    assert_eq!(head_length, HEAD_LEN as u8);
    assert_eq!(url_length, 17);
    assert_eq!(total_length, packed.len() as u32);
    assert_eq!(sequence, 7);
    assert_eq!(
        magic,
        (head_length as usize + 17 + total_length as usize) as u8,
        "`magic` is the low byte of the three lengths added up"
    );
    assert_ne!(hash, 0);

    assert_eq!(&packed[HEAD_LEN..HEAD_LEN + 17], b"/cgi-bin/micromsg");
    assert_eq!(&packed[HEAD_LEN + 17..], b"the body");

    let PackerUnpacked::Ok {
        url,
        sequence,
        pack_len,
        data,
    } = packer_unpack(&packed)
    else {
        panic!("{:?}", packer_unpack(&packed))
    };
    assert_eq!(url, "/cgi-bin/micromsg");
    assert_eq!(sequence, 7);
    assert_eq!(pack_len, packed.len());
    assert_eq!(data, b"the body");
    assert_eq!(packer_unpack(&packed).code(), LONGLINKPACK_OK);
}

#[test]
fn a_package_packed_without_a_hash_is_not_checked_against_one() {
    // `st.hash` stays 0, and `0 != st.hash` is what guards the check
    let packed = packer_pack("/x", 1, b"body", false);
    assert_eq!(head(&packed).6, 0);

    let PackerUnpacked::Ok { data, .. } = packer_unpack(&packed) else {
        panic!("{:?}", packer_unpack(&packed))
    };
    assert_eq!(data, b"body");

    // a body that does not go with the URL, then: with no hash there is nothing
    // to catch it
    let mut wrong = packed.clone();
    let body_at = HEAD_LEN + 2;
    wrong[body_at] = b'X';
    let PackerUnpacked::Ok { .. } = packer_unpack(&wrong) else {
        panic!("a package with no hash is not checked")
    };

    // … and with one it is
    let mut wrong = packer_pack("/x", 1, b"body", true);
    wrong[body_at] = b'X';
    assert_eq!(
        packer_unpack(&wrong),
        PackerUnpacked::Refused(Refused::Hash)
    );
    assert_eq!(packer_unpack(&wrong).code(), LONGLINKPACK_FALSE);
}

#[test]
fn a_url_longer_than_the_c_reads_is_packed_without_its_tail() {
    // `strnlen(_url, 128)`
    let url = "u".repeat(MAX_URL_LEN + 10);
    let packed = packer_pack(&url, 0, b"", true);

    assert_eq!(head(&packed).3, MAX_URL_LEN as u8);
    assert_eq!(packed.len(), HEAD_LEN + MAX_URL_LEN);

    let PackerUnpacked::Ok { url, .. } = packer_unpack(&packed) else {
        panic!("{:?}", packer_unpack(&packed))
    };
    assert_eq!(url, "u".repeat(MAX_URL_LEN));
}

#[test]
fn a_package_of_nothing_is_a_header_and_a_url() {
    let packed = packer_pack("", 0, b"", true);
    assert_eq!(packed.len(), HEAD_LEN);
    assert_eq!(head(&packed).3, 0);
    assert_eq!(head(&packed).4, HEAD_LEN as u32);

    // `NULL == _data` is the same as no body at all
    let PackerUnpacked::Ok { url, data, .. } = packer_unpack(&packed) else {
        panic!("{:?}", packer_unpack(&packed))
    };
    assert_eq!(url, "");
    assert!(data.is_empty());
}

#[test]
fn a_stream_is_read_a_package_at_a_time() {
    let mut stream = packer_pack("/first", 1, b"one", true);
    stream.extend_from_slice(&packer_pack("/second", 2, b"two", true));
    let first_len = packer_pack("/first", 1, b"one", true).len();

    let PackerUnpacked::Ok {
        url,
        sequence,
        pack_len,
        data,
    } = packer_unpack(&stream)
    else {
        panic!("{:?}", packer_unpack(&stream))
    };
    assert_eq!(url, "/first");
    assert_eq!(sequence, 1);
    assert_eq!(pack_len, first_len);
    assert_eq!(data, b"one");

    let PackerUnpacked::Ok {
        url,
        sequence,
        data,
        ..
    } = packer_unpack(&stream[pack_len..])
    else {
        panic!("{:?}", packer_unpack(&stream[pack_len..]))
    };
    assert_eq!(url, "/second");
    assert_eq!(sequence, 2);
    assert_eq!(data, b"two");
}

#[test]
fn a_package_that_has_not_arrived_whole_is_continued() {
    let packed = packer_pack("/x", 3, b"the body", true);

    // fewer bytes than a header
    assert_eq!(
        packer_unpack(&packed[..HEAD_LEN - 1]),
        PackerUnpacked::Continue
    );
    assert_eq!(
        packer_unpack(&packed[..HEAD_LEN - 1]).code(),
        LONGLINKPACK_CONTINUE
    );
    assert_eq!(packer_unpack(&[]), PackerUnpacked::Continue);

    // a header that asks for a URL that is not there yet
    assert_eq!(
        packer_unpack(&packed[..HEAD_LEN + 1]),
        PackerUnpacked::ContinueHead
    );
    assert_eq!(
        packer_unpack(&packed[..HEAD_LEN + 1]).code(),
        LONGLINKPACK_CONTINUE_HEAD
    );

    // the URL is there, the body is not
    let PackerUnpacked::ContinueData {
        url,
        sequence,
        pack_len,
    } = packer_unpack(&packed[..packed.len() - 1])
    else {
        panic!("{:?}", packer_unpack(&packed[..packed.len() - 1]))
    };
    assert_eq!(url, "/x");
    assert_eq!(sequence, 3);
    assert_eq!(pack_len, packed.len());
    assert_eq!(
        packer_unpack(&packed[..packed.len() - 1]).code(),
        LONGLINKPACK_CONTINUE_DATA
    );
}

#[test]
fn a_package_that_does_not_check_out_is_refused() {
    let packed = packer_pack("/x", 1, b"body", true);

    // `magic` — one of the three lengths it is built from
    let mut wrong = packed.clone();
    wrong[0] ^= 0xFF;
    assert_eq!(
        packer_unpack(&wrong),
        PackerUnpacked::Refused(Refused::Magic)
    );

    // the URL and the header do not fit in the length the package claims
    let mut wrong = packed.clone();
    wrong[3] = 200;
    wrong[0] = (HEAD_LEN as u8)
        .wrapping_add(200)
        .wrapping_add(wrong[4..8][3]);
    assert_eq!(
        packer_unpack(&wrong),
        PackerUnpacked::Refused(Refused::Length)
    );

    // bigger than `1024 * 1024`
    let mut wrong = packed.clone();
    wrong[4..8].copy_from_slice(&(1024u32 * 1024 + 1).to_be_bytes());
    wrong[0] = (HEAD_LEN as u8)
        .wrapping_add(2)
        .wrapping_add(wrong[4..8][3]);
    assert_eq!(
        packer_unpack(&wrong),
        PackerUnpacked::Refused(Refused::TooBig)
    );
}

#[test]
fn a_simple_package_is_a_length_and_a_body() {
    // `uint16_t`
    let packed = simple_short_pack(b"hello");
    assert_eq!(simple_short_pack_length(5), 7);
    assert_eq!(packed.len(), 7);
    assert_eq!(&packed[..2], &7u16.to_be_bytes());
    assert_eq!(&packed[2..], b"hello");

    let SimpleUnpacked::Ok { pack_len, data } = simple_short_unpack(&packed) else {
        panic!("{:?}", simple_short_unpack(&packed))
    };
    assert_eq!(pack_len, 7);
    assert_eq!(data, b"hello");
    assert_eq!(simple_short_unpack(&packed).code(), SIMPLE_OK);

    // `uint32_t`
    let packed = simple_int_pack(b"hello");
    assert_eq!(simple_int_pack_length(5), 9);
    assert_eq!(packed.len(), 9);
    assert_eq!(&packed[..4], &9u32.to_be_bytes());

    let SimpleUnpacked::Ok { pack_len, data } = simple_int_unpack(&packed) else {
        panic!("{:?}", simple_int_unpack(&packed))
    };
    assert_eq!(pack_len, 9);
    assert_eq!(data, b"hello");
    assert_eq!(simple_int_unpack(&packed).code(), SIMPLE_OK);
}

#[test]
fn a_simple_package_that_has_not_arrived_whole_is_continued() {
    let packed = simple_short_pack(b"hello");

    // fewer bytes than the length in front of the body
    assert_eq!(simple_short_unpack(&[0]), SimpleUnpacked::Continue);
    assert_eq!(simple_short_unpack(&[0]).code(), SIMPLE_CONTINUE);
    assert_eq!(simple_int_unpack(&[0, 0, 0]), SimpleUnpacked::Continue);

    // the length is there and asks for more
    assert_eq!(
        simple_short_unpack(&packed[..4]),
        SimpleUnpacked::ContinueData { pack_len: 7 }
    );
    assert_eq!(
        simple_short_unpack(&packed[..4]).code(),
        SIMPLE_CONTINUE_DATA
    );
    assert_eq!(
        simple_int_unpack(&simple_int_pack(b"hello")[..6]),
        SimpleUnpacked::ContinueData { pack_len: 9 }
    );
}

#[test]
fn a_simple_package_of_nothing_is_a_length() {
    // `_packlen` counts the length in front of the body, so an empty body is a
    // package of two (or four) bytes
    assert_eq!(simple_short_pack(b"").len(), 2);
    let SimpleUnpacked::Ok { pack_len, data } = simple_short_unpack(&simple_short_pack(b"")) else {
        panic!("{:?}", simple_short_unpack(&simple_short_pack(b"")))
    };
    assert_eq!(pack_len, 2);
    assert!(data.is_empty());
}
