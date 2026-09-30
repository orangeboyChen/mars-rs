//! Runs `tests/cpp_flush.cpp`: the one claim about `include/mars_xlog.hpp` a
//! compile cannot make.
//!
//! `tests/cpp_surface.cpp` is the header compiled and read for symbols, and
//! what that shows is that the C++ is written over the C ABI it says it is. What it cannot
//! show is a `close()` waiting for the drain `flush()` started, because a
//! drain is quick and a compile never runs one. So this test builds the
//! translation unit into a program — the C ABI in it is mocked, and nothing of
//! this library is linked — and runs it. Where there is no C++ compiler the
//! run is skipped, as it is there.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn a_close_waits_for_the_drain_it_was_asked_for() {
    let (Some(cxx), source) = (find_cxx(), translation_unit()) else {
        // No C++ compiler on this machine: `cargo test` is not the place to
        // install one.
        eprintln!("skipped: no C++ compiler among $CXX, c++, clang++ and g++");
        return;
    };

    let out_dir = std::env::temp_dir().join(format!("marsrs-ffi-cpp-flush-{}", std::process::id()));
    fs::create_dir_all(&out_dir).expect("cannot create the directory the program goes in");
    let source_path = out_dir.join("cpp_flush.cpp");
    fs::write(&source_path, source).expect("cannot write the translation unit");
    let program = out_dir.join("cpp_flush");

    // `-pthread` and not a library named: both compilers take the flag, and
    // the thread the drain runs on is the one the header spawns.
    let status = Command::new(&cxx)
        .args(["-std=c++17", "-Wall", "-Wextra", "-pthread"])
        .arg("-I")
        .arg(include_dir())
        .arg(&source_path)
        .arg("-o")
        .arg(&program)
        .status()
        .expect("cannot run the C++ compiler");
    assert!(
        status.success(),
        "`{cxx}` did not build tests/cpp_flush.cpp against include/mars_xlog.hpp"
    );

    let ran = Command::new(&program)
        .status()
        .expect("cannot run the program tests/cpp_flush.cpp was built into");
    assert!(
        ran.success(),
        "tests/cpp_flush.cpp ran and its answer was not success: a `close()` here does not \
         wait for the drain `flush()` started"
    );
}

fn include_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("include")
}

fn translation_unit() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cpp_flush.cpp");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The C++ compiler to build the translation unit with: what `CXX` names, or
/// the first of the three usual ones that answers `--version`.
fn find_cxx() -> Option<String> {
    if let Ok(cxx) = std::env::var("CXX") {
        if !cxx.is_empty() {
            return Some(cxx);
        }
    }
    ["c++", "clang++", "g++"].iter().find_map(|cxx| {
        Command::new(cxx)
            .arg("--version")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|_| (*cxx).to_owned())
    })
}
