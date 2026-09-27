//! Feeds the C++ `__DATE__` / `__TIME__` and `MARS_*` build stamps of
//! `mars/xlog/src/appender.cc` into the crate.
//!
//! `XloggerAppender::Open` writes the banner
//! `"^^^^^^^^^^" __DATE__ "^^^" __TIME__ ...` and `Close` the `$$$` twin, and
//! between them the `MARS_URL` / `MARS_PATH` / `MARS_REVISION` /
//! `MARS_BUILD_TIME` / `MARS_BUILD_JOB` records, so that a log file says which
//! build produced it. Those are compile-time constants of the C++ translation
//! unit; Rust has no `__DATE__`, so this script captures the equivalents when
//! the crate is built and `file_util::build_stamp` reads them back.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let built_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Converted to local date and time by `file_util::build_stamp`, which has
    // `chrono`; a build script may not depend on the crate it builds.
    println!("cargo:rustc-env=MARS_XLOG_BUILD_TIMESTAMP={built_at}");

    println!("cargo:rustc-env=MARS_XLOG_REVISION={}", git_revision());
    println!("cargo:rustc-env=MARS_XLOG_BUILD_JOB={}", build_job());
    println!(
        "cargo:rustc-env=MARS_XLOG_SOURCE_PATH={}",
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| "unknown".to_owned())
    );

    // The stamp is only re-captured when this script changes, which is what
    // `__DATE__` does too: a translation unit keeps its stamp until it is
    // recompiled.
    println!("cargo:rerun-if-changed=build.rs");
}

/// `MARS_REVISION` — the commit the crate was built from.
///
/// `unknown` when git is missing or the checkout is not a repository: a build
/// stamp must never fail the build it describes.
fn git_revision() -> String {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output();
    let Ok(output) = output else {
        return "unknown".to_owned();
    };
    if !output.status.success() {
        return "unknown".to_owned();
    }
    let revision = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if revision.is_empty() {
        "unknown".to_owned()
    } else {
        revision
    }
}

/// `MARS_BUILD_JOB` — the CI run, when there is one.
fn build_job() -> String {
    for var in ["GITHUB_RUN_ID", "CI_JOB_ID", "BUILD_NUMBER"] {
        if let Ok(value) = std::env::var(var) {
            if !value.is_empty() {
                return value;
            }
        }
    }
    "local".to_owned()
}
