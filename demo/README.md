# Demos

One directory per platform the port ships on, and the same demo in every one of
them: open an appender, write one record at each of the six levels, flush, and
say where the file went.

They are written to be read side by side. The point of the port is that one API
is spelled once per platform and means the same thing on all of them —
`Xlog.open(config)` in Rust, in Kotlin, in Swift, in Dart and in ArkTS, and
`mars_xlog_open` in C — so the only thing that differs between these eight is
what a platform makes of two questions: where the log directory comes from, and
whether the call is a future.

| Demo | What it is | Run it with |
| --- | --- | --- |
| [`rust`](rust) | A binary over `marsrs-xlog` | `cargo run` |
| [`c`](c) | A binary over the C ABI | `make run` |
| [`android`](android) | An app over the AAR | `./gradlew installDebug` |
| [`kmp`](kmp) | A Kotlin/Native executable | `./gradlew runDebugExecutableMacosArm64` |
| [`apple`](apple) | An iOS app's sources | Xcode, or drop them into a new app |
| [`flutter`](flutter) | A Flutter app's `lib/` and `pubspec.yaml` | `flutter run` |
| [`react-native`](react-native) | A React Native app's `App.tsx` | `npx react-native run-android` |
| [`harmonyos`](harmonyos) | A DevEco project | `hvigorw assembleHap` |

## What every one of them does

1. **Open.** One appender, with the directory it writes into, the prefix its
   files start with, and the level a record has to reach.
2. **Write.** One record at each level — verbose, debug, info, warning, error
   and fatal — and one more behind an `isLoggable` check, which is the call an
   app makes before it builds a message that is expensive to build.
3. **Flush.** `sync`, so that every record above is on disk when it returns.
   The whole point of the appender is that a write does not block the thread
   that made it, and the whole point of the flush is that the last records are
   not lost when the process goes.
4. **Close.** Which drains what is left.

The file that comes out is `<prefix>_<YYYYMMDD>.xlog`, and
[the CLI](../docs/cli.md) reads it:

```bash
marsrs-xlog-cli decode log/marsrs_20260929.xlog
```

## Which of them are complete projects

The four whose whole toolchain is a command line — `rust`, `c`, `android` and
`kmp` — are complete projects: `cargo run`, `make run`, `./gradlew
installDebug` and `./gradlew runDebugExecutableMacosArm64` build and run them
as they stand, and each is verified by being run.

The other four are complete *sources* and not complete projects, and each one
says so at the top of its own README:

- `apple` needs an Xcode project around it — an iOS app is a bundle, and a
  bundle is not a thing a repository can carry as text. The Swift package of
  the port declares iOS and watchOS and no macOS, so there is no macOS
  executable to build either.
- `flutter` and `react-native` need the platform folders their own CLIs
  generate: `flutter create` and `@react-native-community/cli init` write a
  hundred files each, Gradle projects and Xcode projects among them, and none
  of them is a file this repository should keep.
- `harmonyos` is a whole hvigor project, and the one thing it cannot carry is
  the HAR of the port — publishing to ohpm is not switched on yet, so it is
  downloaded out of the release.

## The versions

`android` and `kmp` are Gradle builds in the Kotlin DSL — `.gradle.kts` for the
scripts, `gradle/libs.versions.toml` for the coordinates — on the newest Gradle,
and the version of the port each takes is one line in that catalog. Bump the
line, not the build.
