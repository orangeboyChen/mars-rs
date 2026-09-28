# HarmonyOS

Two ways in, and the difference is who writes the NAPI module: `marsrs-harmonyos-xlog`
is the package — a HAR an app installs and writes ArkTS through — and
`marsrs-harmony-<version>.tar.gz` is the three `libmars_ffi.so` and the header
for an app that would rather write NAPI of its own. Take one of the two, not
both: both carry the same Rust core, and an app with two copies of the appender
in one process has two appenders writing the same file.

## Install

```bash
ohpm install marsrs-harmonyos-xlog
```

ohpm is not published to yet, so until it is, a release is where the package
comes from: take `marsrs-harmonyos-xlog-<version>.har` off it and install that
file.

```bash
ohpm install ./marsrs-harmonyos-xlog-<version>.har
```

```text
marsrs-harmonyos-xlog-<version>.har
    Index.ets                       what `import { Xlog } from …` resolves
    oh-package.json5
    src/main/ets/xlog/Xlog.ets      the ArkTS
    src/main/cpp/napi_init.cpp      the NAPI module it calls
    libs/arm64-v8a/libmarsrs_xlog.so
    libs/armeabi-v7a/libmarsrs_xlog.so
    libs/x86_64/libmarsrs_xlog.so
```

`libmarsrs_xlog.so` is `libmars_ffi` — the staticlib of the C ABI — with
`napi_init.cpp` linked around it, so an app that takes the HAR resolves nothing
else. That shim is C and not C++, and that matters to an app: a NAPI module that
reaches for `std::mutex` asks the process loading it for a C++ runtime it would
otherwise never load. What ships is a binary and not a native project the app's
build has to run.

The NAPI module is `marsrs_xlog`, which is the name `import xlogNapi from
'libmarsrs_xlog.so'` resolves, and the declarations of what it exports are
`src/main/cpp/types/libmarsrs_xlog/index.d.ts` — the file hvigor finds where it
is, and the reason no `types` field is needed in `oh-package.json5`.

## Open, write, flush

```typescript
import { AppenderMode, CompressMode, LogLevel, Xlog } from 'marsrs-harmonyos-xlog';

const xlog: Xlog = Xlog.open({
  logDir: `${getContext().filesDir}/xlog/log`,
  cacheDir: `${getContext().filesDir}/xlog/cache`,
  namePrefix: 'marsrs',
  level: LogLevel.Info,
  mode: AppenderMode.Async,
  compressMode: CompressMode.Zlib,
});
xlog.consoleLogEnabled = true;

xlog.i('startup', 'hello from mars');

xlog.flush(true);   // before the app reads or uploads the files
xlog.close();
```

`logDir` is the one option with no default — the rest are on
[the configuration page](/configuration), under the names the Kotlin of the port
gives them. `Xlog.open` throws when the appender will not take the directory: an
empty `logDir`, or one the process cannot write to — which is what the React
Native module does, and the Kotlin too.

`logDir` under `getContext().filesDir` is what an app wants: the directory the
app owns, which the appender can create and the [log files
page](/log-files) names out of.

## Nothing answers a `Promise`

Every method of the NAPI module is synchronous, so a call returns from the
thread that made it and answers no promise — the record is in the C ABI before
the next line runs. That is what lets this `Xlog` be the `Xlog` of
`platforms/android/marsrs` and of `platforms/react-native/marsrs-xlog` member
for member, and not the Flutter's, whose calls cross a channel and answer a
`Future`.

The five settings are properties and not `setLevel` / `getLevel` pairs, because
a property is the spelling the Swift and the Kotlin use:

```typescript
xlog.level = LogLevel.Warning;      // the appender's own, read back through it
xlog.maxFileSizeBytes = 8 * 1024 * 1024;
```

`level` is the one of the five the C ABI answers a getter for, so `xlog.level`
reads the appender's own; `mode`, `consoleLogEnabled`, `maxFileSizeBytes` and
`maxAliveTimeSeconds` answer what this instance last wrote, which is all the C
ABI leaves to answer — and all the Swift and the Kotlin of the port answer too.

`LogLevel` is `Verbose` / `Debug` / `Info` / `Warning` / `Error` / `Fatal` /
`None`, PascalCase rather than the Kotlin's `INFO`: an ArkTS enum is written in
an app full of HarmonyOS enums, and this is the spelling they use. What crosses
the ABI either way is the C ABI's own integer.

## Writing

The write is `android.util.Log`'s shape — `v`/`d`/`i`/`w`/`e`/`f`, each of them a
tag and a message, and `log(level, tag, message)` when the level is not known
until the call.

```typescript
xlog.v('net', '…');
xlog.d('net', '…');
xlog.i('startup', '…');
xlog.w('net', '…');
xlog.e('login', '…');
xlog.f('login', '…');

xlog.log(LogLevel.Debug, 'net', '…');
```

A record below the level the appender was opened at is dropped before anything is
formatted. A message that is expensive to build is worth asking about first —
the record is dropped either way, and what `isLoggable` saves is the string:

```typescript
if (xlog.isLoggable(LogLevel.Debug)) {
  xlog.d('net', expensiveDescription());
}
```

## While it is open

| what | how |
|---|---|
| move the level | `xlog.level = LogLevel.Warning` |
| read the level back | `xlog.level` |
| switch async / sync | `xlog.mode = AppenderMode.Sync` |
| mirror records to the console | `xlog.consoleLogEnabled = true` |
| close a file at a size | `xlog.maxFileSizeBytes = 8 * 1024 * 1024` |
| drop a file at an age | `xlog.maxAliveTimeSeconds = 10 * 24 * 3600` |
| is it still open | `xlog.isOpen` |
| drain the cache | `xlog.flush(true)` |

`close()` drains what is left and closes the appender. Two `Xlog`s of one
`namePrefix` are one appender — the native side holds one handle per prefix, and
every call carries the prefix it is about — and an `Xlog.open` of a prefix that
is already open answers the appender it made rather than a second one over it,
so two names hold one `Xlog` and `close` on either closes it for both. A part of
the app whose logs are read apart from the rest wants a prefix of its own.

## The other way in: the three `.so`

An app that writes NAPI of its own takes `marsrs-harmony-<version>.tar.gz`
instead, and gets the C ABI with nothing wrapped around it:

```text
marsrs-harmony-<version>/
    arm64-v8a/libmars_ffi.so       aarch64-unknown-linux-ohos
    armeabi-v7a/libmars_ffi.so     armv7-unknown-linux-ohos
    x86_64/libmars_ffi.so          x86_64-unknown-linux-ohos
    include/mars_xlog.h
```

Drop `libmars_ffi.so` under the module's `libs/<abi>/` — `arm64-v8a` for a real
device — and open the appender from C, once, when the app starts:

```c
#include <mars_xlog.h>

MarsXLogConfig config = {
    .mode = MarsAppenderAsync,
    .log_dir = log_dir,      /* the app's files directory */
    .name_prefix = "marsrs",
    .compress_mode = MarsCompressZlib,
};
mars_xlog_open(&config);
mars_xlog_write(MarsLevelInfo, "startup", __FILE__, __func__, __LINE__, "hello");
```

Everything on [the C ABI page](/platforms/c-abi) applies: the config, the
levels, the instances and the error codes are the same symbols.

## The mars half

`libmars_ffi.so` is the whole port and not the logger alone: it is built with the
`sdt` and `stn` features on top of the default `xlog`, and what sits next to the
library is all three of `mars_xlog.h`, `mars_sdt.h` and `mars_stn.h`. So a HarmonyOS app can run a task, or a
diagnosis, through the same NAPI shim it writes for the logger:

```c
#include <mars_stn.h>

mars_stn_set_app(NULL, ask);          /* the eighteen questions, as one callback */
mars_stn_start_task(&task);

/* the answer is how many milliseconds the pass may wait */
long long due = mars_stn_due_time();
while (due >= 0) {                    /* the app drains the queues */
    usleep(due * 1000);
    mars_stn_run_pending();
    due = mars_stn_due_time();
}
```

No ArkTS of this package reaches either half — the HAR is the logger — and the
two pages say what each one asks of an app: [the task pipeline](/stn) and
[the network diagnosis](/sdt). Nothing there is HarmonyOS's own — it is the same
C ABI [its page](/platforms/c-abi) publishes for Linux, macOS and Windows, and
the one `MarsRSNet`, the Android AARs and `marsrs-kmp` are written over.

## What is not in it

The file, the function and the line of a record are empty: there is no ArkTS
frame worth naming in a record, and the C++ writes an empty one too. Where the
current file is is not answered — no `mars_xlog_current_log_path` and no
`Xlog.currentLogPath` — because the path is a thing the app asks of the
directory it gave, and [the log files page](/log-files) is what names it.
