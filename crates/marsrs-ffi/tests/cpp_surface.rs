//! Keeps `include/mars_xlog.hpp` — the C++ face of this seam — in step with the
//! C it is written over.
//!
//! Two things can go wrong in a header no Rust code compiles, and neither of
//! them is a thing `cargo build` would notice: it can name a symbol
//! `mars_xlog.h` does not declare, and it can ask the linker for a *mangled*
//! name for one the library exports unmangled, which is what happens the moment
//! the `extern "C"` guard of `mars_xlog.h` is lost. The first is a text check;
//! the second needs a compiler, so where there is none the compile is skipped
//! and the text check is what is left.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Every `mars_xlog_*` the C++ header is written over: the one member of
/// [`marsrs::xlog::Xlog`](../../include/mars_xlog.hpp) each of them is.
const SYMBOLS: [&str; 14] = [
    "mars_xlog_new_instance",
    "mars_xlog_get_instance",
    "mars_xlog_release_instance",
    "mars_xlog_get_level",
    "mars_xlog_set_level_instance",
    "mars_xlog_set_mode_instance",
    "mars_xlog_set_console_log_instance",
    "mars_xlog_set_max_file_size_instance",
    "mars_xlog_set_max_alive_duration_instance",
    "mars_xlog_is_enabled_for",
    "mars_xlog_write_instance",
    "mars_xlog_request_flush_instance",
    "mars_xlog_flush_now_instance",
    "mars_xlog_set_console_fun",
];

fn include_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("include")
}

fn read_include(name: &str) -> String {
    let path = include_dir().join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn translation_unit() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cpp_surface.cpp");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

#[test]
fn the_cpp_header_names_only_symbols_the_c_header_declares() {
    let hpp = read_include("mars_xlog.hpp");
    let h = read_include("mars_xlog.h");
    for symbol in SYMBOLS {
        assert!(
            hpp.contains(symbol),
            "include/mars_xlog.hpp calls `{symbol}`, which include/mars_xlog.h does not declare"
        );
        assert!(
            h.contains(symbol),
            "include/mars_xlog.h is missing `{symbol}`, which include/mars_xlog.hpp calls"
        );
    }
    assert!(
        h.contains("extern \"C\""),
        "include/mars_xlog.h has lost its `extern \"C\"` guard: a C++ caller now asks its \
         linker for a mangled name the library does not export"
    );
}

#[test]
fn the_cpp_header_compiles_and_asks_for_the_c_symbols() {
    let (Some(cxx), source) = (find_cxx(), translation_unit()) else {
        // No C++ compiler on this machine: `cargo test` is not the place to
        // install one, and the text check above is what is left.
        eprintln!("skipped: no C++ compiler among $CXX, c++, clang++ and g++");
        return;
    };

    let out_dir =
        std::env::temp_dir().join(format!("marsrs-ffi-cpp-surface-{}", std::process::id()));
    fs::create_dir_all(&out_dir).expect("cannot create the directory the object file goes in");
    let source_path = out_dir.join("cpp_surface.cpp");
    fs::write(&source_path, source).expect("cannot write the translation unit");
    let object = out_dir.join("cpp_surface.o");

    // `-Wall -Wextra` and not `-Werror`: what one compiler warns about is not
    // what the next one does, and this test is about the header compiling.
    let status = Command::new(&cxx)
        .args(["-std=c++17", "-Wall", "-Wextra"])
        .arg("-I")
        .arg(include_dir())
        .arg("-c")
        .arg(&source_path)
        .arg("-o")
        .arg(&object)
        .status()
        .expect("cannot run the C++ compiler");
    assert!(
        status.success(),
        "`{cxx}` did not compile tests/cpp_surface.cpp against include/mars_xlog.hpp"
    );

    let Some(listed) = listed_undefined(&object) else {
        eprintln!("skipped the symbol check: neither `llvm-nm` nor `nm` listed the object file");
        return;
    };

    for symbol in SYMBOLS {
        assert!(
            listed.lines().any(|line| line.contains(symbol)),
            "`{symbol}` is not among the symbols tests/cpp_surface.cpp asks for"
        );
    }
    for line in listed.lines() {
        assert!(
            !(line.contains("_Z") && line.contains("mars_xlog")),
            "`{line}` is a mangled name: the `extern \"C\"` guard of include/mars_xlog.h is gone, \
             so a C++ caller asks its linker for a symbol the library does not export"
        );
    }
}

/// The C++ compiler to compile the translation unit with: what `CXX` names, or
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

/// The symbols an object file asks for and does not define: `llvm-nm` and `nm`
/// both take `-u`, and neither is a thing a Rust test can assume is installed.
fn listed_undefined(object: &Path) -> Option<String> {
    ["llvm-nm", "nm"]
        .iter()
        .find_map(|nm| {
            Command::new(nm)
                .arg("-u")
                .arg(object)
                .output()
                .ok()
                .filter(|output| output.status.success())
        })
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
}
