//! The `xlog` CLI, driven end to end: every combination of compressor, sync
//! and key it can be given, and the ways a command line is refused.
//!
//! What it cannot check on its own is the one thing the format is for — that a
//! file the **C++** implementation wrote decodes to the text that went in.
//! `crates/marsrs-compat` checks that over the golden files of its `fixtures/`,
//! and reads through the same `decode_records` this CLI does, so the two
//! halves cover one reader between them.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

    // `-e` and `-d` spelled the way a flag is, every short option there is, and
    // a value attached to its letter: `-oFILE`.
    let attached = dir.join("attached.xlog");
    let (ok, _, err) = run(&[
        "-e",
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
    let (ok, out, err) = run(&["-d", &attached.display().to_string()]);
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
            "-o, --out",
            "-k, --privkey",
            "-p, --pubkey",
        ] {
            assert!(usage.contains(word), "the usage does not mention `{word}`");
        }
    }
}
