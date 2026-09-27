# Android

Two AARs over the same `libmarsxlog.so` — the pair the C++ project publishes
as `mars-core` and `mars-xlog`:

```kotlin
// settings.gradle.kts
maven { url = uri("https://jitpack.io") }

// build.gradle.kts
implementation("io.github.orangeboychen:mars-rs:0.1.0")       // the whole port
implementation("io.github.orangeboychen:mars-rs-xlog:0.1.0")  // xlog alone
```

| AAR | artifact | what is in it |
|---|---|---|
| `mars-core.aar` | `mars-rs` | every Kotlin class whose natives `marsrs-jni` exports — `xlog`, `stn`, `sdt`, `app`, `comm` and `BaseEvent`/`Mars` — plus `libmarsxlog.so` for `arm64-v8a`, `armeabi-v7a` and `x86_64` |
| `mars-xlog.aar` | `mars-rs-xlog` | `xlog.Xlog` — with `XlogConfig`, `LogLevel`, `AppenderMode` and `CompressMode` — and the `xlog.Log` facade over it, plus the same `libmarsxlog.so` |

Take `mars-rs` when you want STN or SDT, `mars-rs-xlog` when the app only logs;
both carry the whole library, because there is one `.so` and it is not split.
The Kotlin and Java package is `io.github.orangeboychen.marsrs`, the package
whose natives `marsrs-jni` exports, so the two are renamed together. The AAR's face
is Kotlin — `Xlog`, `Log`, `StnLogic`, `SdtLogic` and the rest — written so that
an app in Java sees the same statics the C++ project's Java had. A repository is also
reachable on JitPack as `com.github.<owner>.<repo>`, which is the spelling its
badge prints; the release workflow asks jitpack.io to build the tag under both,
reports which one answered, and then checks that both AARs resolve.

JitPack has an Android SDK but neither an NDK nor a Rust toolchain, so it
downloads `mars-android-native.zip` of the same release first — see
`jitpack.yml`.

## Using it

Two steps: build the appender once when the app starts, then write through it
from wherever there is something to say.

```kotlin
val xlog = Xlog(
    XlogConfig(
        logDir = File(context.filesDir, "xlog/log").path,
        cacheDir = File(context.filesDir, "xlog/cache").path,
        namePrefix = "Ham",
        level = LogLevel.INFO,
        mode = AppenderMode.ASYNC,
    )
)
xlog.consoleLogEnabled = BuildConfig.DEBUG

xlog.i("startup", "cold start in $elapsedMillis ms")
xlog.e("login", "login failed\n${cause.stackTraceToString()}")
```

The write is `android.util.Log`'s shape — `v`/`d`/`i`/`w`/`e`/`f`, each of them
a tag and a message, and `log(level, tag, message)` when the level is not known
until the call — so Java writes `new Xlog(config)` and `xlog.i(tag, message)`
with nothing else to learn. A record costs one JNI call: `marsrs-jni` is what
drops a record the appender's level is above, before anything is formatted, and
what fills the pid and the tid in from the OS. A message that is expensive to
build is worth an `if (xlog.isLoggable(LogLevel.DEBUG))` first — a record the
level drops costs the caller the `String` either way.

`XlogConfig` is the Kotlin face of the appender's options — `LogLevel`,
`AppenderMode` and `CompressMode` are enums over the numbers `marsrs-jni` speaks
— and it refuses a config the `.so` cannot honour here, in Kotlin, because
`marsrs-jni` answers a config it does not like by opening nothing. A part of an
app whose logs are read apart from the rest gets an appender of its own out of
a second `Xlog(XlogConfig(...))` with a `namePrefix` of its own, and
`xlog.flush(sync = true)` before the app reads or uploads its files: a record of
the default `ASYNC` mode sits in a memory-mapped cache until a writer thread
takes it to the log file.

`Xlog` also still carries what the C++ project's Java spelled — the
seven-argument `open`, `XLogConfig`, `XLoggerInfo`, `logWrite`, the `LEVEL_*`
constants and the `Log` facade — so an app that already calls any of them keeps
working, and every one of them is deprecated with the spelling that replaces
it. `Xlog.open` installs `Xlog` into `Log` as well, so the two spellings write
through one appender and land in the same file, and a migration can go one call
site at a time.
