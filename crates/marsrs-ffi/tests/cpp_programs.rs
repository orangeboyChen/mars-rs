//! Runs the C++ programs under `tests/`: the claims about
//! `include/mars_xlog.hpp` that a compile cannot make.
//!
//! `tests/cpp_surface.cpp` is the header compiled and read for symbols, and
//! what that shows is that the C++ is written over the C ABI it says it is.
//! What it cannot show is what the header *does* — whether a `close()` waits
//! for the drain `flush()` started, and whether it releases the appender its
//! own handle names and not the one the prefix answers by the time it asks —
//! because a compile never runs one. So each program here is built and run:
//! the C ABI in it is mocked, and nothing of this library is linked, so the
//! drain can be made slow and the registry can answer the way it does when two
//! threads race. Where there is no C++ compiler the runs are skipped, as they
//! are there.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn a_close_waits_for_the_drain_it_was_asked_for() {
    run(
        "cpp_flush",
        "a `close()` here does not wait for the drain `flush()` started",
    );
}

#[test]
fn a_close_releases_the_appender_its_handle_names() {
    run(
        "cpp_release",
        "a `close()` here does not release the appender its own handle names",
    );
}

/// Builds `name.cpp` into a program and runs it: what it answers is success,
/// or a line on stdout saying which promise of the header it found broken.
fn run(name: &str, failure: &str) {
    let (Some(cxx), source) = (find_cxx(), translation_unit(name)) else {
        // No C++ compiler on this machine: `cargo test` is not the place to
        // install one.
        eprintln!("skipped: no C++ compiler among $CXX, c++, clang++ and g++");
        return;
    };

    let out_dir =
        std::env::temp_dir().join(format!("marsrs-ffi-cpp-{name}-{}", std::process::id()));
    fs::create_dir_all(&out_dir).expect("cannot create the directory the program goes in");
    let source_path = out_dir.join(format!("{name}.cpp"));
    fs::write(&source_path, source).expect("cannot write the translation unit");
    let program = out_dir.join(name);

    // `-pthread` and not a library named: both compilers take the flag, and
    // the thread a drain runs on is one the header spawns.
    let built = Command::new(&cxx)
        .args(["-std=c++17", "-Wall", "-Wextra", "-pthread"])
        .arg("-I")
        .arg(include_dir())
        .arg(&source_path)
        .arg("-o")
        .arg(&program)
        .status()
        .expect("cannot run the C++ compiler");
    assert!(
        built.success(),
        "`{cxx}` did not build tests/{name}.cpp against include/mars_xlog.hpp"
    );

    let ran = Command::new(&program)
        .status()
        .expect("cannot run the program the translation unit was built into");
    assert!(
        ran.success(),
        "tests/{name}.cpp ran and its answer was not success: {failure}"
    );
}

fn include_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("include")
}

fn translation_unit(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/{name}.cpp"));
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The C++ compiler to build the translation units with: what `CXX` names, or
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
