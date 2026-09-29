# The Flutter demo

A Flutter app's own two files — `pubspec.yaml` and `lib/main.dart` — and not the
whole project: the platform folders are Flutter's to generate, and `flutter
create` writes a hundred files, Gradle projects and Xcode projects among them,
that no repository should keep.

```bash
flutter create --org io.github.orangeboychen --project-name marsrs_demo .
flutter pub add marsrs_xlog path_provider
flutter run
```

`flutter create` leaves `pubspec.yaml` and `lib/` alone and writes the runners
around them, so the two files here survive it — run it from this directory, and
this README is the only thing it may overwrite.

## What to look at

- `Xlog.open(config)` — a future, unlike Kotlin's and Swift's: the appender
  lives behind a method channel, so opening one is a round trip to the platform
  and `main` awaits it before the first frame.
- `xlog.i(tag, message)` — *not* a future. A write hands the record to the
  appender and returns; `await xlog.flush()` is the call an app awaits before it
  reads the files or uploads them.
- `xlog.isLoggable(level)` — the one check that is a future, because it asks the
  appender itself rather than a field this side holds.
- `getApplicationSupportDirectory()` — the directory the platform gives the app,
  on Android and on iOS alike. `getTemporaryDirectory()` is the one the OS
  empties when space runs short, which is exactly when a log matters.

`marsrs_xlog` is the plugin an app that only logs takes; `marsrs` is the whole
port. An app takes one of the two and never both — both carry the same native
library, and an app with two of it does not build.
