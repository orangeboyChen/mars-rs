# Build, test and lint

```bash
cargo build --workspace
cargo test  --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Cross-compiling needs the usual toolchain plus, for Android, an NDK (only for
the linker):

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
export ANDROID_HOME=...            # .cargo/config.toml forces 16 KiB pages
cargo build --release -p marsrs-jni --target aarch64-linux-android
```

`unsafe` appears in exactly three places: the C ABI shims of `marsrs-ffi`,
the single `memmap2::MmapOptions::map_mut` call of `marsrs-appender` (there
is no safe API for creating a mapping) and the one `JString::from_raw` of
`marsrs-jni`, which re-wraps a borrowed local ref. All three carry SAFETY notes.

## Lint

One gate per language, and each of them fails on the first finding:
`.github/workflows/lint-swift.yml` and `lint-kotlin.yml` run the Swift and the
Kotlin gate, `rust.yml` runs the Rust one. Every run starts, but a job of it
skips its work when the commit touches nothing that job reads: a pull request
with no Swift in it does not pay for a macOS runner, and one with no Kotlin in
it does not pay for a JDK. The filter is on the jobs and not on the run because
a workflow that never starts reports no status at all, and the checks the
ruleset requires would be left waiting for one — as #108 was, for two days —
while a skipped job answers them.

| Language | Tools | Configuration |
| --- | --- | --- |
| Rust | `rustfmt`, `clippy` | `rustfmt.toml`, `clippy.toml`, `[workspace.lints]` of `Cargo.toml` |
| Swift | SwiftLint 0.65.1 | `.swiftlint.yml` |
| Kotlin | ktlint 1.8.0, detekt 1.23.8 | `.editorconfig`, `detekt.yml` |

```bash
# Swift: `--strict` turns every warning into an error.
swiftlint lint --strict

# Kotlin: the style and the line length of .editorconfig, then the analysis
# detekt.yml configures on top of detekt's own defaults.
ktlint --relative 'android/**/*.kt' 'android/**/*.kts'
java -jar detekt-cli-1.23.8-all.jar \
  --input android/mars-core/src/main/kotlin,android/mars-xlog/src/main/kotlin \
  --config detekt.yml --build-upon-default-config
```

The tool versions are pinned, and each of the three configurations says what it
turns on beyond the tool's own defaults and why — `detekt.yml` in particular is
an override file: every rule it names is either a threshold the default sets too
low or one the port cannot satisfy without changing what it does, and it says
which. Every finding is answered in the source or it fails the job, so there is
no suppression comment anywhere: the constants of the AAR's Kotlin are spelled
the way Kotlin spells a constant — `kPingCheck` of the C++ project's Java is
`K_PING_CHECK` here — rather than kept under a `ktlint-disable`, and the JNI
reaches a constant by the number it carries, not by its name. `detekt.yml` is
the only place a rule a tool turns on by default is off, and each of the seven
says why. The twelve crates are held to the same rule: no `#[allow]` answers a lint
in them, and the one `#[allow]` left in the tree is `unsafe_code` on the single
`mmap` of `marsrs-appender` — a crate that denies `unsafe_code` outright, with the
invariant the mapping needs argued beside it.

## The site

The documentation you are reading is a VitePress site under `docs/`:

```bash
cd docs
npm ci
npm run dev      # http://localhost:5173/mars-rs/
npm run build    # into docs/.vitepress/dist
```

`.github/workflows/docs.yml` builds it and publishes it to GitHub Pages.
