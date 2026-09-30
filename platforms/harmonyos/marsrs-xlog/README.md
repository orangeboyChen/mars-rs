# marsrs-harmonyos-xlog

xlog — the logging half of [mars-rs](https://github.com/orangeboyChen/mars-rs) —
for a HarmonyOS app: the `.xlog` the C++ Tencent/mars writes, written from
ArkTS.

What it writes is what the C++ implementation writes — same framing, same
compression, same encryption — so the decoders that came with upstream, and any
tooling built on them, read these files without a conversion step.

## Install

```bash
ohpm install marsrs-harmonyos-xlog
```

The package carries `libmarsrs_xlog.so` for `arm64-v8a`, `armeabi-v7a` and
`x86_64`, with `libmars_ffi` — the Rust core — linked into it. There is nothing
else to resolve.

## Use it

Open an appender once when the app starts, write through it, and flush before
the app reads or uploads its files:

```typescript
import { Xlog, LogLevel } from 'marsrs-harmonyos-xlog';

const xlog: Xlog = Xlog.open({
  logDir: `${getContext().filesDir}/xlog/log`,
  cacheDir: `${getContext().filesDir}/xlog/cache`,
  namePrefix: 'marsrs',
  level: LogLevel.Info,
});
xlog.consoleLogEnabled = true;

xlog.i('startup', 'cold start');
xlog.e('login', 'login failed');

xlog.flushNow();    // before the app reads or uploads the files
```

The file is `<logDir>/<namePrefix>_YYYYMMDD.xlog` —
`marsrs_20260927.xlog` above. The default mode is async, so a record can sit in
the cache for a moment: `flushNow()` before the file is read or uploaded —
`requestFlush()` only tells the writer thread it may drain, and guarantees
nothing when it returns.

## The API

The same one the Kotlin, the Swift, the Kotlin Multiplatform and the React
Native packages carry, member for member:

| | |
|---|---|
| `Xlog.open(config)` | opens the appender of `config` |
| `xlog.v/d/i/w/e/f(tag, message)` | writes at that level |
| `xlog.log(level, tag, message)` | writes at a level of your own |
| `xlog.level`, `.mode`, `.consoleLogEnabled`, `.maxFileSizeBytes`, `.maxAliveTimeSeconds` | the five settings, as properties |
| `xlog.isLoggable(level)` | whether a record at `level` would be written |
| `xlog.requestFlush()` | tells the writer thread it may take the cache to the file, and returns at once |
| `xlog.flushNow()` | takes the cache to the file on the calling thread; the records are on disk when it returns |
| `xlog.close()` | closes the appender |
| `xlog.isOpen` | whether it is still open |

What the file being written is, and what a whole day of them are:

| | |
|---|---|
| `xlog.currentLogPath` | the file this appender is writing to; `undefined` until the day's first record opens one |
| `xlog.logFiles(daysAgo)` | that day's files that are *there* — `0` is today, `1` is yesterday |
| `xlog.logFileNames(daysAgo)` | that day's paths, whether or not they are there yet |

A day is asked about by this appender's prefix and directory, and not by a
handle, so the answer is the same whether or not the appender is open. `open` is
the only static of the class: there is no process-wide appender here to ask about,
and an app that opens one `Xlog` per prefix drains each of them with its own
`flushNow()`.

`Xlog.open` of a `namePrefix` that is already open answers the appender that is
open and not a second one: the native side is one appender per prefix, so two
`Xlog`s of one prefix are one appender, and `close` on either closes both.

## License

MIT, like the upstream project.
