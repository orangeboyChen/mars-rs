//! The `xlog` CLI, driven end to end: every combination of compressor, sync
//! and key it can be given, the pair `keygen` makes, and the ways a command
//! line is refused.
//!
//! What it cannot check on its own is the one thing the format is for — that a
//! file the **C++** implementation wrote decodes to the text that went in.
//! `crates/marsrs-compat` checks that over the golden files of its `fixtures/`,
//! and reads through the same `decode_records` this CLI does, so the two
//! halves cover one reader between them.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use marsrs_crypt::{HEADER_LEN, TAILER_LEN};

/// The pair of `crates/marsrs-compat/fixtures/manifest.json` — the public key
/// the C++ encoders were configured with, and the private key every decoder of
/// those files needs.
const PUBKEY: &str = "196d0b02543df00c8c1cf8ac58540286d3e568d7316a5de8a2f5a46f44a712518c726bb538f6abbc970391160c35f2df4c30bdb1a04cdba75c6098dcf2b4734b";
const PRIVKEY: &str = "ef7e9da1abac66449ba6f8697d4d0ac63fecd5679da53b9aab1f72aba73d8e2b";

/// One record per line, and a multi-byte one: a decoder that mangles the
/// payload still compares equal after a lossy conversion, so every comparison
/// below is a comparison of bytes and not of strings.
const RECORDS: &[u8] = b"hello xlog\n[2026-09-23 09:12:33][I][marsrs] first record\n\
                        \xe4\xb8\xad\xe6\x96\x87\xe6\x97\xa5\xe5\xbf\x97 compatibility\n";

fn xlog() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xlog"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("marsrs-xlog-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write the scratch file");
    path
}

/// Writes an hour no magic is spelt with over the two hour bytes of every
/// header in `bytes`.
///
/// A header carries the hour of the day the record was written at, and
/// `getLogStartPos` takes any byte from `MAGIC_CRYPT_START` — `0x01` — on as
/// the start of a record, so a file written at an hour of 1 to 13 — one in the
/// morning to one in the afternoon — holds a byte in its own header that the
/// walk resyncs into when it goes looking for the record behind the damage.
/// What the tests below assert is the damage they put in and not where the
/// walk came out of it, so the hour is `0`: no magic of any kind, and the one
/// `marsrs_compat::normalize_for_compare` writes over both of them with as
/// well, so that a run that crosses an hour is not a file that changed.
fn no_magic_hours(bytes: &mut [u8]) {
    let mut offset = 0;
    while offset + HEADER_LEN + TAILER_LEN <= bytes.len() {
        bytes[offset + 3] = 0;
        bytes[offset + 4] = 0;
        let length = u32::from_le_bytes(
            bytes[offset + 5..offset + 9]
                .try_into()
                .expect("four bytes of length"),
        ) as usize;
        offset += HEADER_LEN + length + TAILER_LEN;
    }
}

/// Runs `xlog` with `args` and returns whether it succeeded, what it wrote to
/// standard output, and what it said on standard error.
fn run(args: &[&str]) -> (bool, Vec<u8>, String) {
    let output = xlog().args(args).output().expect("run the CLI");
    (
        output.status.success(),
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The same, in `dir`: what a relative path on the command line is relative
/// to, which is the machine's own working directory and not one of these
/// tests'.
fn run_in(dir: &Path, args: &[&str]) -> (bool, Vec<u8>, String) {
    let output = xlog()
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run the CLI");
    (
        output.status.success(),
        output.stdout,
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn every_mode_round_trips_the_records() {
    let dir = scratch("round-trip");
    let input = write(&dir, "records.txt", RECORDS);

    for mode in ["zlib", "zstd"] {
        for sync in ["0", "1"] {
            for pubkey in [None, Some(PUBKEY)] {
                let name = format!(
                    "{mode}-sync{sync}-{}",
                    if pubkey.is_some() { "crypt" } else { "clear" }
                );
                let file = dir.join(format!("{name}.xlog"));
                let plain = dir.join(format!("{name}.plain"));

                let mut args = vec![
                    "encode".to_owned(),
                    format!("--mode={mode}"),
                    format!("--sync={sync}"),
                ];
                if let Some(key) = pubkey {
                    args.push(format!("--pubkey={key}"));
                }
                args.push(input.display().to_string());
                args.push("--out".to_owned());
                args.push(file.display().to_string());
                let args: Vec<&str> = args.iter().map(String::as_str).collect();

                let (ok, _, err) = run(&args);
                assert!(ok, "{name} could not be encoded: {err}");
                assert!(
                    std::fs::metadata(&file)
                        .expect("the .xlog was written")
                        .len()
                        > 0,
                    "{name} is empty"
                );

                // An encrypted file is only readable with the private key; a
                // clear one is readable with none, which is the same claim.
                let mut args = vec!["decode".to_owned()];
                if pubkey.is_some() {
                    args.push(format!("--privkey={PRIVKEY}"));
                }
                args.push(file.display().to_string());
                args.push("--out".to_owned());
                args.push(plain.display().to_string());
                let args: Vec<&str> = args.iter().map(String::as_str).collect();

                let (ok, _, err) = run(&args);
                assert!(ok, "{name} could not be decoded: {err}");
                assert_eq!(
                    std::fs::read(&plain).expect("read the decoded text"),
                    RECORDS,
                    "{name} did not give the records back"
                );
            }
        }
    }
}

#[test]
fn an_encrypted_file_is_not_read_without_the_private_key() {
    let dir = scratch("no-key");
    let input = write(&dir, "records.txt", RECORDS);
    let file = dir.join("a.xlog");

    let (ok, _, err) = run(&[
        "encode",
        &format!("--pubkey={PUBKEY}"),
        &input.display().to_string(),
        "--out",
        &file.display().to_string(),
    ]);
    assert!(ok, "encode failed: {err}");

    let out = dir.join("out");
    let (ok, _, err) = run(&[
        "decode",
        &file.display().to_string(),
        "--out",
        &out.display().to_string(),
    ]);
    assert!(!ok, "a file was decoded without the key");
    assert!(
        err.contains("is encrypted"),
        "the error does not say why: {err}"
    );
    assert!(!out.exists(), "the text was written anyway");
}

/// The two halves of what `xlog keygen` printed, `pubkey=` and `privkey=`.
fn pair(out: &[u8]) -> (String, String) {
    let mut pubkey = None;
    let mut privkey = None;
    for line in String::from_utf8_lossy(out).lines() {
        match line.split_once('=') {
            Some(("pubkey", hex)) => pubkey = Some(hex.to_owned()),
            Some(("privkey", hex)) => privkey = Some(hex.to_owned()),
            _ => {}
        }
    }
    (pubkey.unwrap_or_default(), privkey.unwrap_or_default())
}

#[test]
fn a_key_pair_is_made_and_reads_what_it_wrote() {
    let dir = scratch("keygen");
    let input = write(&dir, "records.txt", RECORDS);
    let file = dir.join("a.xlog");

    let (ok, out, err) = run(&["keygen"]);
    assert!(ok, "keygen failed: {err}");
    let (pubkey, privkey) = pair(&out);
    assert_eq!(pubkey.len(), 128, "`{pubkey}` is not 128 hex characters");
    assert_eq!(privkey.len(), 64, "`{privkey}` is not 64 hex characters");

    // The pair is one the appender's own handshake is over: `encode` takes the
    // public key — which is what says it is a key of the right curve, and not a
    // string of the right length — and the private key is the only thing that
    // gives the records back.
    let (ok, _, err) = run(&[
        "encode",
        &format!("--pubkey={pubkey}"),
        &input.display().to_string(),
        "--out",
        &file.display().to_string(),
    ]);
    assert!(ok, "encode with the pair keygen made failed: {err}");
    let (ok, out, err) = run(&[
        "decode",
        &format!("--privkey={privkey}"),
        &file.display().to_string(),
    ]);
    assert!(ok, "decode with the pair keygen made failed: {err}");
    assert_eq!(out, RECORDS, "the records did not come back");

    // And it is a fresh pair and not a fixed one: two runs agree on nothing, so
    // a pair is written down and not remembered.
    let (ok, second, err) = run(&["keygen"]);
    assert!(ok, "the second keygen failed: {err}");
    let (again, _) = pair(&second);
    assert_ne!(again, pubkey, "two runs made the same public key");
}

#[test]
fn the_key_pair_goes_where_out_says() {
    let dir = scratch("keygen-out");
    let key = dir.join("xlog.key");

    // `xlog k` is `xlog keygen`, and `--out` the one option it takes.
    let (ok, out, err) = run(&["k", "-o", &key.display().to_string()]);
    assert!(ok, "keygen failed: {err}");
    assert!(
        out.is_empty(),
        "the pair went to the terminal as well as the file"
    );

    let written = std::fs::read_to_string(&key).expect("read the key file");
    let (pubkey, privkey) = pair(written.as_bytes());
    assert_eq!(pubkey.len(), 128, "the file does not hold a public key");
    assert_eq!(privkey.len(), 64, "the file does not hold a private key");
}

#[test]
fn the_key_file_is_the_owners_alone_and_is_not_written_over() {
    let dir = scratch("keygen-file");
    let key = dir.join("xlog.key");

    let (ok, _, err) = run(&["keygen", "-o", &key.display().to_string()]);
    assert!(ok, "keygen failed: {err}");
    let first = std::fs::read_to_string(&key).expect("read the key file");

    // The private key is not given away with the file: on Unix it is created
    // for its owner alone, and not with the `0666 & umask` a plain `fs::write`
    // would leave it with.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = std::fs::metadata(&key)
            .expect("the key file")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "the private key is readable by more than its owner"
        );
    }

    // And it is not replaced: a pair is the only thing that reads every log it
    // was the key of, so a second run over the same file is refused.
    let (ok, _, err) = run(&["keygen", "-o", &key.display().to_string()]);
    assert!(!ok, "a second keygen overwrote the first pair");
    assert!(
        err.contains("is already there"),
        "the error does not say why: {err}"
    );
    assert_eq!(
        std::fs::read_to_string(&key).expect("read the key file"),
        first,
        "the first pair was replaced"
    );
}

/// `--out` of `decode` is the log text itself, so it is created the way the key
/// file is — for its owner alone, and not for every user of the machine.
#[test]
fn the_decoded_log_goes_to_a_file_of_the_owners_alone() {
    let dir = scratch("decode-out");
    let input = write(&dir, "records.txt", RECORDS);
    let file = dir.join("a.xlog");
    let plain = dir.join("a.plain");

    let (ok, _, err) = run(&[
        "encode",
        &input.display().to_string(),
        "--out",
        &file.display().to_string(),
    ]);
    assert!(ok, "encode failed: {err}");
    let (ok, _, err) = run(&[
        "decode",
        &file.display().to_string(),
        "--out",
        &plain.display().to_string(),
    ]);
    assert!(ok, "decode failed: {err}");
    assert_eq!(
        std::fs::read(&plain).expect("read the decoded text"),
        RECORDS,
        "the records did not come back"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = std::fs::metadata(&plain)
            .expect("the decoded file")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "the decoded log is readable by more than its owner"
        );
    }
}

#[test]
fn keygen_takes_no_other_option_and_no_input() {
    // An option of another subcommand: the answer names the one that takes it.
    for (arg, why) in [
        (
            format!("--pubkey={PUBKEY}"),
            "--pubkey is an option of `xlog encode`",
        ),
        (
            format!("--privkey={PRIVKEY}"),
            "--privkey is an option of `xlog decode`",
        ),
    ] {
        let (ok, _, err) = run(&["keygen", &arg]);
        assert!(!ok, "`{arg}` was accepted by keygen");
        assert!(err.contains(why), "the error does not say why: {err}");
    }

    // An option no subcommand of this one has: a `--mode` or a `--region` a key
    // pair has nothing to do with.
    for arg in ["--mode=zstd", "--region=4096", "--in=records.txt"] {
        let (ok, _, err) = run(&["keygen", arg]);
        assert!(!ok, "`{arg}` was accepted by keygen");
        assert!(
            err.contains("not an option of `xlog keygen`"),
            "the error does not say why: {err}"
        );
    }

    // And an input: `keygen` makes a key of its own and reads nothing.
    let (ok, _, err) = run(&["keygen", "a.xlog"]);
    assert!(!ok, "an input was accepted by keygen");
    assert!(
        err.contains("takes no input"),
        "the error does not say why: {err}"
    );
}

#[test]
fn a_record_that_declares_more_than_the_file_holds_is_reported() {
    let dir = scratch("declared-length");
    let input = write(&dir, "records.txt", RECORDS);
    let file = dir.join("a.xlog");

    let (ok, _, err) = run(&[
        "encode",
        &input.display().to_string(),
        "-o",
        &file.display().to_string(),
    ]);
    assert!(ok, "encode failed: {err}");

    // The length field of the header says `u32::MAX`: a reader that adds it to
    // the offset wraps on a 32-bit target and panics on the slice, so what is
    // checked is the length against what is left of the input.
    let mut bytes = std::fs::read(&file).expect("read the .xlog");
    no_magic_hours(&mut bytes);
    bytes[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
    let broken = write(&dir, "broken.xlog", &bytes);

    let (ok, _, err) = run(&["decode", &broken.display().to_string()]);
    assert!(!ok, "a record that is not in the file was decoded");
    assert!(
        err.contains("truncated"),
        "the error does not say why: {err}"
    );
}

#[test]
fn a_public_key_that_is_not_one_is_refused_before_anything_is_written() {
    let dir = scratch("bad-key");
    let input = write(&dir, "records.txt", RECORDS);
    let file = dir.join("a.xlog");

    let (ok, _, err) = run(&[
        "encode",
        "--pubkey=deadbeef",
        &input.display().to_string(),
        "--out",
        &file.display().to_string(),
    ]);
    assert!(!ok, "a file was written with a key nobody holds");
    assert!(
        err.contains("128 hex"),
        "the error does not say what a key is: {err}"
    );
    assert!(!file.exists(), "the .xlog was written anyway");
}

#[test]
fn standard_input_and_output_are_the_defaults() {
    let mut child = xlog()
        .args(["encode", "--mode=zstd"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the CLI");
    child
        .stdin
        .take()
        .expect("the stdin pipe")
        .write_all(RECORDS)
        .expect("write the records");
    let encoded = child.wait_with_output().expect("the .xlog");
    assert!(encoded.status.success(), "encode failed");
    assert!(!encoded.stdout.is_empty(), "the .xlog went nowhere");

    let mut child = xlog()
        .arg("decode")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the CLI");
    child
        .stdin
        .take()
        .expect("the stdin pipe")
        .write_all(&encoded.stdout)
        .expect("write the .xlog");
    let decoded = child.wait_with_output().expect("the log text");
    assert!(decoded.status.success(), "decode failed");
    assert_eq!(decoded.stdout, RECORDS, "the records did not come back");
}

/// An `=` with nothing after it is the stream, and not a value that has to be
/// looked for in the next argument: `decode -o= a.xlog` reads `a.xlog` and
/// prints it to the terminal, where `-o` followed by `a.xlog` decodes what is
/// on standard input and writes it over the file the command line named.
#[test]
fn an_equals_with_nothing_after_it_is_the_stream() {
    let dir = scratch("empty-value");
    let input = write(&dir, "records.txt", RECORDS);
    let file = dir.join("a.xlog");

    let (ok, _, err) = run(&[
        "encode",
        &input.display().to_string(),
        "--out",
        &file.display().to_string(),
    ]);
    assert!(ok, "encode failed: {err}");
    let before = std::fs::read(&file).expect("read the .xlog");

    let (ok, out, err) = run(&["decode", "-o=", &file.display().to_string()]);
    assert!(ok, "decode failed: {err}");
    assert_eq!(out, RECORDS, "the records did not come back");
    assert_eq!(
        std::fs::read(&file).expect("read the .xlog again"),
        before,
        "the file it was given was written over"
    );
}

#[test]
fn an_uncompressed_async_record_is_refused_and_a_sync_one_is_not() {
    let dir = scratch("uncompressed");
    let input = write(&dir, "records.txt", RECORDS);
    let file = dir.join("a.xlog");

    // An async body is framed as zlib or zstd whatever `--compress` says, so
    // `--compress=0` would write a file that decodes to nothing at all.
    let (ok, _, err) = run(&[
        "encode",
        "--compress=0",
        &input.display().to_string(),
        "-o",
        &file.display().to_string(),
    ]);
    assert!(!ok, "a file that decodes to nothing was written");
    assert!(
        err.contains("--sync=1"),
        "the error does not say what to do instead: {err}"
    );
    assert!(!file.exists(), "the .xlog was written anyway");

    // `--sync=1` is the mode that stores a record verbatim, which is what
    // `--compress=0` asks for.
    let (ok, _, err) = run(&[
        "encode",
        "--compress=0",
        "--sync=1",
        &input.display().to_string(),
        "-o",
        &file.display().to_string(),
    ]);
    assert!(ok, "encode failed: {err}");
    let (ok, out, err) = run(&["decode", &file.display().to_string()]);
    assert!(ok, "decode failed: {err}");
    assert_eq!(out, RECORDS, "the records did not come back");
}

/// `count` records of `len` bytes each, all of one length: in sync mode a
/// record is a block of its own, so a file of them is `count` blocks of one
/// size and where a block starts is arithmetic and not a guess.
fn fat_records(count: usize, len: usize) -> Vec<u8> {
    let mut text = Vec::new();
    for index in 0..count {
        // The index padded to `len - 1` characters, then the newline a record
        // carries: `len` bytes, whatever the index is.
        text.extend_from_slice(format!("{index:0>width$}", width = len - 1).as_bytes());
        text.push(b'\n');
    }
    text
}

#[test]
fn a_file_cut_short_still_yields_the_records_before_the_cut() {
    let dir = scratch("cut-short");
    let records = fat_records(5, 200);
    let input = write(&dir, "records.txt", &records);
    let file = dir.join("a.xlog");

    // `--sync=1` is one block per record: five of them, and the fifth is the
    // one the cut lands in.
    let (ok, _, err) = run(&[
        "encode",
        "--sync=1",
        &input.display().to_string(),
        "-o",
        &file.display().to_string(),
    ]);
    assert!(ok, "encode failed: {err}");

    let mut bytes = std::fs::read(&file).expect("read the .xlog");
    no_magic_hours(&mut bytes);
    let stride = bytes.len() / 5;
    // Halfway into the last block, past its header: what a process killed
    // between two writes leaves in the file.
    let cut = write(&dir, "cut.xlog", &bytes[..4 * stride + 150]);

    let (ok, out, err) = run(&["decode", &cut.display().to_string()]);
    assert!(!ok, "a file that is missing its end decoded as a whole one");
    assert!(
        err.contains("truncated"),
        "the error does not say why: {err}"
    );
    // The four records in front of the cut are the four fifths of the input:
    // they are the answer to what the file says, and dropping them for the
    // sake of the fifth is what this did before.
    assert_eq!(
        out,
        records[..4 * 200].to_vec(),
        "the records before the cut were dropped too"
    );
}

/// `count` records of sixteen hex characters, from a xorshift: input no
/// compressor shrinks by much, so a region really does fill up.
fn random_records(count: usize) -> Vec<u8> {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut text = String::new();
    for _ in 0..count {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        text.push_str(&format!("{state:016x}"));
        text.push('\n');
    }
    text.into_bytes()
}

/// One record out of `count` of them — the newlines taken out and one put back
/// at the end.
fn one_random_record(count: usize) -> Vec<u8> {
    let mut text = String::from_utf8(random_records(count)).expect("the records are hex");
    text.retain(|char| char != '\n');
    text.push('\n');
    text.into_bytes()
}

#[test]
fn a_region_smaller_than_the_records_round_trips_them_anyway() {
    let dir = scratch("small-region");
    let records = random_records(400);
    let input = write(&dir, "records.txt", &records);
    let file = dir.join("a.xlog");

    // A region that holds a handful of records and not the whole input: the
    // file is one block per flush, and every record is in one of them.
    for region in ["256", "4096"] {
        let (ok, _, err) = run(&[
            "encode",
            &format!("--region={region}"),
            &input.display().to_string(),
            "-o",
            &file.display().to_string(),
        ]);
        assert!(ok, "encode with --region={region} failed: {err}");
        let (ok, out, err) = run(&["decode", &file.display().to_string()]);
        assert!(ok, "decode with --region={region} failed: {err}");
        assert_eq!(
            out, records,
            "--region={region} dropped part of the records"
        );
    }

    // And a record longer than the region asked for: the region is raised to
    // what the largest record needs instead of the record being refused.
    let one = write(&dir, "one.txt", &one_random_record(512));
    let (ok, _, err) = run(&[
        "encode",
        "--region=64",
        &one.display().to_string(),
        "-o",
        &file.display().to_string(),
    ]);
    assert!(ok, "encode of one long record failed: {err}");
    let (ok, out, err) = run(&["decode", &file.display().to_string()]);
    assert!(ok, "decode failed: {err}");
    assert_eq!(out, std::fs::read(&one).expect("read the input"));
}

/// Bytes no compressor can shrink.
///
/// What the size of the region is raised to is computed from the length of the
/// largest record and from the most a compressor adds to an input like that,
/// so a record a compressor only makes longer is the one that asks the whole
/// question: a region sized without the block's header is a region too short
/// by [`HEADER_LEN`], and [`LogBuffer::write`] drops what does not fit without
/// saying so.
fn incompressible_record(len: usize) -> Vec<u8> {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut bytes = Vec::with_capacity(len + 1);
    while bytes.len() < len {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mixed = (state ^ (state >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        bytes.extend_from_slice(&mixed.to_le_bytes());
    }
    bytes.truncate(len);
    bytes.push(b'\n');
    bytes
}

#[test]
fn a_record_a_compressor_cannot_shrink_is_written_whole() {
    let dir = scratch("incompressible");
    for mode in ["zlib", "zstd"] {
        for len in [512usize, 2048] {
            let record = incompressible_record(len);
            let input = write(&dir, &format!("record-{mode}-{len}.txt"), &record);
            let file = dir.join(format!("a-{mode}-{len}.xlog"));
            let (ok, _, err) = run(&[
                "encode",
                &format!("--mode={mode}"),
                &format!("--region={len}"),
                &input.display().to_string(),
                "-o",
                &file.display().to_string(),
            ]);
            assert!(ok, "encode of a {len} byte record failed: {err}");
            let (ok, out, err) = run(&["decode", &file.display().to_string()]);
            assert!(ok, "decode of a {len} byte record failed: {err}");
            assert_eq!(out, record, "the tail of a {len} byte record was dropped");
        }
    }
}

#[test]
fn every_subcommand_and_option_has_a_short_spelling() {
    let dir = scratch("short");
    let input = write(&dir, "records.txt", RECORDS);
    let file = dir.join("a.xlog");
    let plain = dir.join("a.plain");

    // `xlog e -p <key> -i <input> -o <file>` is `xlog encode --pubkey=… --in=…
    // --out=…`, and `-o=<file>` is the third spelling of the same option.
    let (ok, _, err) = run(&[
        "e",
        "-p",
        PUBKEY,
        "-i",
        &input.display().to_string(),
        "-o",
        &file.display().to_string(),
    ]);
    assert!(ok, "encode failed: {err}");

    let (ok, _, err) = run(&[
        "d",
        "-k",
        PRIVKEY,
        &file.display().to_string(),
        &format!("-o={}", plain.display()),
    ]);
    assert!(ok, "decode failed: {err}");
    assert_eq!(
        std::fs::read(&plain).expect("read the decoded text"),
        RECORDS,
        "the short spelling wrote something else"
    );

    // `e` and `d`, every short option there is, and a value attached to its
    // letter: `-oFILE`.
    let attached = dir.join("attached.xlog");
    let (ok, _, err) = run(&[
        "e",
        "-m",
        "zstd",
        "-c1",
        "-s0",
        "-l3",
        "-r4096",
        &input.display().to_string(),
        &format!("-o{}", attached.display()),
    ]);
    assert!(ok, "encode with an attached value failed: {err}");
    let (ok, out, err) = run(&["d", &attached.display().to_string()]);
    assert!(ok, "decode failed: {err}");
    assert_eq!(out, RECORDS, "the records did not come back");
}

#[test]
fn a_broken_command_line_is_refused() {
    // A subcommand that is not one.
    let (ok, _, err) = run(&["frobnicate"]);
    assert!(!ok);
    assert!(
        err.contains("unknown subcommand"),
        "the error does not say why: {err}"
    );

    // A dash makes an option and not a subcommand: `xlog -e` is refused, and
    // `xlog e` is the command.
    let (ok, _, err) = run(&["-e", "-o", "a.xlog"]);
    assert!(!ok, "`xlog -e` was read as `xlog e`");
    assert!(
        err.contains("unknown option"),
        "the error does not say why: {err}"
    );

    // An option that is not one, in either spelling.
    for arg in ["--nonsense", "-z"] {
        let (ok, _, err) = run(&["decode", arg, "a.xlog"]);
        assert!(!ok, "`{arg}` was accepted");
        assert!(
            err.contains("unknown option"),
            "the error does not say why: {err}"
        );
    }

    // An option of the other subcommand: a `--privkey` that `encode` would
    // ignore writes a file nobody asked for.
    let (ok, _, err) = run(&["encode", &format!("--privkey={PRIVKEY}")]);
    assert!(!ok);
    assert!(
        err.contains("option of `xlog decode`"),
        "the error does not say why: {err}"
    );

    // An option whose value is missing.
    let (ok, _, err) = run(&["decode", "--privkey"]);
    assert!(!ok);
    assert!(
        err.contains("needs a value"),
        "the error does not say what is missing: {err}"
    );

    // Two inputs.
    let dir = scratch("two-inputs");
    let first = write(&dir, "a", RECORDS);
    let second = write(&dir, "b", RECORDS);
    let (ok, _, err) = run(&[
        "encode",
        &first.display().to_string(),
        &second.display().to_string(),
    ]);
    assert!(!ok);
    assert!(
        err.contains("two inputs"),
        "the error does not say why: {err}"
    );
}

#[test]
fn an_input_with_no_record_in_it_is_refused() {
    let dir = scratch("empty");
    let input = write(&dir, "records.txt", b"");
    let (ok, _, err) = run(&[
        "encode",
        &input.display().to_string(),
        "--out",
        &dir.join("a.xlog").display().to_string(),
    ]);
    assert!(!ok);
    assert!(
        err.contains("no record"),
        "the error does not say why: {err}"
    );
}

#[test]
fn the_version_and_the_usage_are_printed() {
    for arg in ["--version", "-V"] {
        let (ok, out, _) = run(&[arg]);
        assert!(ok, "`{arg}` failed");
        let version = String::from_utf8_lossy(&out).into_owned();
        assert!(version.starts_with("xlog "), "`{version}` is not a version");
    }

    for arg in ["help", "h", "--help", "-h"] {
        let (ok, out, _) = run(&[arg]);
        assert!(ok, "`{arg}` failed");
        let usage = String::from_utf8_lossy(&out).into_owned();
        for word in [
            "xlog encode",
            "xlog decode",
            "xlog keygen",
            "-o, --out",
            "-k, --privkey",
            "-p, --pubkey",
        ] {
            assert!(usage.contains(word), "the usage does not mention `{word}`");
        }
    }
}

/// A file read with the private key of another pair is a file no record of
/// which comes out: the walk goes on from a record it cannot read, so what it
/// answers with is a marker per record and no log at all. A command that
/// answers well to that is a command that read nothing.
#[test]
fn a_file_read_with_the_key_of_another_pair_is_refused() {
    let dir = scratch("wrong-key");
    let input = write(&dir, "records.txt", RECORDS);
    let file = dir.join("a.xlog");

    let (ok, _, err) = run(&[
        "encode",
        &format!("--pubkey={PUBKEY}"),
        &input.display().to_string(),
        "-o",
        &file.display().to_string(),
    ]);
    assert!(ok, "encode failed: {err}");

    // A key of another pair: 64 hex characters, and not the one the file was
    // written with.
    let (ok, out, err) = run(&["keygen"]);
    assert!(ok, "keygen failed: {err}");
    let (_, other) = pair(&out);

    let (ok, out, err) = run(&[
        "decode",
        &format!("--privkey={other}"),
        &file.display().to_string(),
    ]);
    assert!(!ok, "a file was decoded with the key of another pair");
    assert!(
        err.contains("came out"),
        "the error does not say what happened: {err}"
    );
    // The markers are written out anyway — a file that did not read is still a
    // file the operator asked to see — but they are not the log: what each of
    // them says is why that record did not come out.
    let text = String::from_utf8_lossy(&out).into_owned();
    assert!(!out.is_empty(), "no marker was written");
    assert!(text.contains("error"), "no reason was written: {text}");
    assert_ne!(out, RECORDS, "the records came out with the wrong key");
}

/// The usage is asked for as a subcommand, or as a flag of its own — and not
/// from the value of an option, which is an argument like any other.
#[test]
fn an_options_value_is_not_the_question_it_looks_like() {
    let dir = scratch("value-spelling");
    let input = write(&dir, "records.txt", RECORDS);

    // `--out help` names a file called `help`.
    let (ok, out, err) = run_in(
        &dir,
        &["encode", &input.display().to_string(), "--out", "help"],
    );
    assert!(ok, "encode to a file called `help` failed: {err}");
    assert!(
        out.is_empty(),
        "the usage was printed instead: {}",
        String::from_utf8_lossy(&out)
    );
    let help = dir.join("help");
    assert!(
        std::fs::metadata(&help)
            .expect("a file called `help`")
            .len()
            > 0,
        "no file was written"
    );

    // And `--help` of a subcommand is the question, wherever it stands.
    for args in [["encode", "--help"].as_slice(), ["decode", "-h"].as_slice()] {
        let (ok, out, _) = run(args);
        assert!(ok, "`{}` failed", args.join(" "));
        let usage = String::from_utf8_lossy(&out).into_owned();
        assert!(
            usage.contains("xlog keygen"),
            "no usage was printed: {usage}"
        );
    }
}

/// An input whose name starts with a dash is an input: what says so is the
/// `--` that ends the options.
#[test]
fn a_dash_dash_ends_the_options() {
    let dir = scratch("dash-dash");
    write(&dir, "-weird.txt", RECORDS);

    // Without it, an option parser takes the name for an option of its own:
    // `w` is no option of any subcommand's.
    let (ok, _, err) = run_in(&dir, &["encode", "-weird.txt"]);
    assert!(!ok, "a name of that shape was read as an input");
    assert!(
        err.contains("unknown option"),
        "the error does not say why: {err}"
    );

    // The options stand in front of the `--`: everything behind it is an input.
    let (ok, _, err) = run_in(&dir, &["encode", "-o", "a.xlog", "--", "-weird.txt"]);
    assert!(
        ok,
        "encode of a file whose name starts with a dash failed: {err}"
    );
    let (ok, out, err) = run_in(&dir, &["decode", "--", "a.xlog"]);
    assert!(ok, "decode failed: {err}");
    assert_eq!(out, RECORDS, "the records did not come back");
}

/// A value of no characters is the stream the option names: `--out=` is
/// standard output, and `--in=` standard input. A path of no characters is not
/// one anything can open.
#[test]
fn an_empty_value_is_the_stream_it_names() {
    let dir = scratch("empty-value");
    let input = write(&dir, "records.txt", RECORDS);
    let file = dir.join("a.xlog");

    let (ok, _, err) = run(&[
        "encode",
        &input.display().to_string(),
        "-o",
        &file.display().to_string(),
    ]);
    assert!(ok, "encode failed: {err}");

    let (ok, out, err) = run(&["decode", &file.display().to_string(), "--out="]);
    assert!(ok, "decode with an empty --out failed: {err}");
    assert_eq!(out, RECORDS, "the records did not come back");

    let mut child = xlog()
        .args(["decode", "--in="])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the CLI");
    child
        .stdin
        .take()
        .expect("the stdin pipe")
        .write_all(&std::fs::read(&file).expect("read the .xlog"))
        .expect("write the .xlog");
    let decoded = child.wait_with_output().expect("the log text");
    assert!(
        decoded.status.success(),
        "decode from standard input failed: {}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    assert_eq!(decoded.stdout, RECORDS, "the records did not come back");
}

/// The region is `vec![0u8; region_len]`, and `--region` is a floor and not a
/// ceiling — a record that needs more room is given it — so a number past the
/// largest one there is a typo and not a request.
#[test]
fn a_region_past_the_largest_one_is_refused() {
    let dir = scratch("region-cap");
    let input = write(&dir, "records.txt", RECORDS);

    let (ok, _, err) = run(&[
        "encode",
        "--region=999999999999",
        &input.display().to_string(),
    ]);
    assert!(!ok, "a terabyte of region was accepted");
    assert!(
        err.contains("at most"),
        "the error does not say what the largest one is: {err}"
    );
}
