//! The CLI, driven end to end — including the case that matters most: decoding
//! a file the **C++** implementation wrote, with the encryption on.

use std::path::PathBuf;
use std::process::Command;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn manifest() -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(fixtures().join("manifest.json")).unwrap())
        .unwrap()
}

fn xlog_compat() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xlog-compat"))
}

#[test]
fn decodes_an_encrypted_file_the_cpp_wrote() {
    let manifest = manifest();
    let privkey = manifest["privkey"].as_str().unwrap();
    let out = std::env::temp_dir().join(format!("cli-decode-{}.plain", std::process::id()));
    let file = "zlib-async-crypt1-flush0.xlog";

    let status = xlog_compat()
        .args([
            "decode",
            &format!("--privkey={privkey}"),
            &format!("--in={}", fixtures().join(file).display()),
            &format!("--out={}", out.display()),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "{file} could not be decoded");

    let expected = std::fs::read(fixtures().join("expected.bin")).unwrap();
    let got = std::fs::read(&out).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&got),
        String::from_utf8_lossy(&expected),
        "{file} did not decode back to the text that went in"
    );
    // the encrypted fixture must really be encrypted: the plaintext is not
    // visible in the file itself
    let raw = std::fs::read(fixtures().join(file)).unwrap();
    let needle = expected
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default()
        .to_vec();
    if !needle.is_empty() {
        assert!(
            !raw.windows(needle.len()).any(|window| window == needle),
            "the plaintext is stored in the clear"
        );
    }
    std::fs::remove_file(&out).ok();
}

#[test]
fn decodes_every_golden_fixture() {
    let manifest = manifest();
    let privkey = manifest["privkey"].as_str().unwrap().to_owned();
    let expected = std::fs::read(fixtures().join("expected.bin")).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let file = case["file"].as_str().unwrap();
        let out = std::env::temp_dir().join(format!("cli-{}-{}.plain", std::process::id(), file));
        let status = xlog_compat()
            .args([
                "decode",
                &format!("--privkey={privkey}"),
                &format!("--in={}", fixtures().join(file).display()),
                &format!("--out={}", out.display()),
            ])
            .status()
            .unwrap();
        assert!(status.success(), "{file} failed");
        let got = std::fs::read(&out).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&got),
            String::from_utf8_lossy(&expected),
            "{file} differs"
        );
        std::fs::remove_file(&out).ok();
    }
}

#[test]
fn encodes_and_round_trips_through_the_cli() {
    let records = fixtures().join("inputs.bin");
    let out = std::env::temp_dir().join(format!("cli-encode-{}.xlog", std::process::id()));
    let status = xlog_compat()
        .args([
            "encode",
            "--mode=zlib",
            "--compress=1",
            "--sync=0",
            &format!("--records={}", records.display()),
            &format!("--out={}", out.display()),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "encode failed");
    assert!(out.exists());

    // and the same file decodes back
    let back = std::env::temp_dir().join(format!("cli-encode-{}.plain", std::process::id()));
    let output = xlog_compat()
        .args([
            "decode",
            &format!("--privkey={}", manifest()["privkey"].as_str().unwrap()),
            &format!("--in={}", out.display()),
            &format!("--out={}", back.display()),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "the encoded file did not decode: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_file(&back).ok();
    std::fs::remove_file(&out).ok();
}

#[test]
fn rejects_a_bad_invocation() {
    let output = xlog_compat().args(["nonsense"]).output().unwrap();
    assert!(!output.status.success());
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        said.contains("nonsense"),
        "the CLI said nothing about the bad subcommand: {said}"
    );
}

#[test]
fn reports_a_missing_file() {
    let output = xlog_compat()
        .args(["decode", "--in=/definitely/not/here.xlog", "--out=/tmp/x"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}
