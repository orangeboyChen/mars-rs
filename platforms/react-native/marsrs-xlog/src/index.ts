// The TypeScript face of the xlog half of mars-rs.
//
// A TurboModule, and not a bridge module: `src/NativeXlog.ts` is the spec
// codegen reads, and what it answers is a module whose methods are called where
// they are asked for and return before the next line runs — one that answers no
// promise is a call and not an `await`, and one that answers a value answers it
// before it returns. So this `Xlog` is the `Xlog` of
// `platforms/apple/MarsRSXlog/Xlog.swift`, of
// `platforms/android/marsrs-xlog` and of `platforms/kmp/marsrs-xlog` member
// for member: `Xlog.open(config)` answers the appender, `xlog.level` is the
// property it is in Kotlin and in Swift, and `xlog.i(tag, message)` has
// landed by the time it returns.
//
// Five settings are properties and not `setLevel` / `getLevel` pairs, because a
// JS property is the Kotlin and the Swift spelling and a TurboModule setter is
// called where it is written: `xlog.level` reads the appender's own and writes
// it, and `mode`, `consoleLogEnabled`, `maxFileSizeBytes` and
// `maxAliveTimeSeconds` answer what this instance last wrote — the C ABI has no
// getter for those four, and neither the Kotlin nor the Swift answers one.

import NativeXlog from './NativeXlog';

/** `TLogLevel`; `none` is `MARS_LEVEL_NONE`, which the filter understands but
 * the C enum does not carry. */
export const LogLevel = {
  verbose: 0,
  debug: 1,
  info: 2,
  warning: 3,
  error: 4,
  fatal: 5,
  none: 6,
} as const;

export type LogLevel = (typeof LogLevel)[keyof typeof LogLevel];

/** `TAppenderMode`: whether a record reaches the file before the call returns. */
export const AppenderMode = {
  /** The record goes into the memory-mapped cache and a writer thread takes it
   * to the file — what the C++ opens with, and what `flushNow()` and
   * `await flush()` drain. */
  async: 0,
  /** The record is in the file before the call returns, at a write and a lock
   * per line. */
  sync: 1,
} as const;

export type AppenderMode = (typeof AppenderMode)[keyof typeof AppenderMode];

/** `TCompressMode`: how a log file is compressed once it is closed. */
export const CompressMode = {
  /** zlib, what the C++ opens with and what every reader of a mars log file
   * understands. */
  zlib: 0,
  /** Zstandard: smaller at the same speed, and unreadable by a tool that only
   * knows zlib. */
  zstd: 1,
} as const;

export type CompressMode = (typeof CompressMode)[keyof typeof CompressMode];

/** `XLogConfig` of the C++ project, in the spelling the Kotlin of the port
 * gives it: what an `Xlog` is opened with.
 *
 * Every field has the default the C++'s own `XLogConfig` carries, so the one an
 * app has to give is `logDir` — the C ABI opens no appender
 * without it. */
export interface XlogConfig {
  /** Where the log files are written; created if it is missing, and the one
   * field with no default. */
  logDir: string;
  /** What every log file starts with (`marsrs_20260927.xlog`), and the name the
   * appender is known by — an app that writes through two of them gives them
   * two. `xlog` when left out, when handed over empty, and when what was
   * handed over is nothing but whitespace. */
  namePrefix?: string;
  /** The level the appender is opened at; `info` when left out. */
  level?: LogLevel;
  /** Whether a write reaches the file before it returns. */
  mode?: AppenderMode;
  /** Empty means the log is written unencrypted. */
  pubKey?: string;
  /** How a closed log file is compressed. */
  compressMode?: CompressMode;
  /** `0` keeps the appender's own default (6). */
  compressLevel?: number;
  /** Left out, the mmap cache lives in the log directory. */
  cacheDir?: string;
  /** `0` keeps every file. */
  cacheDays?: number;
}

/** `XlogConfig.namePrefix` of the Kotlin and of the Swift: what an appender is
 * opened with when a caller gives none — and when it gives a blank one, which
 * the two halves do not agree about: the Kotlin answers its own default for a
 * prefix of nothing but whitespace, and the Swift opens that prefix as it is.
 * Naming the default here is what keeps the `Xlog` an app holds and the
 * appender the halves hold the same one, whichever half it is — a prefix of
 * spaces that one half substituted and the other did not is an appender this
 * `Xlog` cannot ask about afterwards. */
const DEFAULT_NAME_PREFIX = 'xlog';

/** The shortest lifetime of a file the appender takes, in seconds: one below it
 * — `0` among them — is refused, and the appender keeps the lifetime it had.
 * `MIN_LOG_ALIVE_TIME` of the Rust, which is the one place the number is
 * written down. */
const MIN_ALIVE_TIME_SECONDS = 86400;

/** The prefix an appender opened with `config` is known by: `namePrefix`, or
 * [DEFAULT_NAME_PREFIX] when it is missing or blank. What is handed to the two
 * halves is this and not the string the caller gave, so the two of them agree
 * on one name for one appender. */
function namePrefixOf(config: XlogConfig): string {
  return config.namePrefix?.trim() || DEFAULT_NAME_PREFIX;
}

/** The appender of every `namePrefix` `Xlog.open` has opened and `close` has
 * not closed, by the prefix: what makes two `Xlog.open`s of one prefix one
 * appender, which the native side already is — one module, one appender per
 * prefix, and every call carrying the prefix it is about.
 *
 * The second `open` answers the appender the first made, so the two names an
 * app holds are one `Xlog`: `close` on either is `close` on both, and nothing
 * writes through an appender that is gone — which is what it would do, and in
 * silence, if the second call were a second instance the first one's `close`
 * did not know about. */
const openAppenders = new Map<string, Xlog>();

/** The appender of one `namePrefix`: the `Xlog` of the Swift and the Kotlin of
 * the port. `Xlog.open` makes it, and `close` releases the appender it made. */
export class Xlog {
  /** The name the appender is known by, and what every one of its files starts
   * with: the `namePrefix` of the Kotlin and of the Swift. */
  readonly namePrefix: string;

  private currentLevel: LogLevel;

  private currentMode: AppenderMode;

  private currentConsoleLogEnabled = false;

  private currentMaxFileSize = 0;

  private currentMaxAliveTime = 0;

  /** Whether `close` has been called on this instance. */
  private open = true;

  private constructor(config: XlogConfig) {
    this.namePrefix = namePrefixOf(config);
    this.currentLevel = config.level ?? LogLevel.info;
    this.currentMode = config.mode ?? AppenderMode.async;
  }

  /** `mars_xlog_new_instance`: opens the appender of `config` and answers the
   * `Xlog` that writes through it.
   *
   * The appender of a `namePrefix` this has already opened is answered as it
   * is, and not opened again: the native side is one appender per prefix, and a
   * second one over the first would be a handle nothing releases. So two calls
   * of one prefix are one `Xlog`, and `close` on it is `close` on both.
   *
   * The native side is asked either way, and not only the first time: one
   * appender per prefix is one for the whole app, so one the app closed
   * through its own Kotlin or Swift is gone from under this map, and asking
   * again is what opens it again.
   *
   * Throws when the appender would not take the configuration — an empty
   * `logDir`, or a directory it cannot write to. */
  static open(config: XlogConfig): Xlog {
    const namePrefix = namePrefixOf(config);
    const alreadyOpen = openAppenders.get(namePrefix);
    // The appender of a prefix is one for the whole process, so one another
    // part of the app closed is gone from under this map: the open is asked
    // again instead of trusted, and what it answers is the appender that is
    // there now — the same one when nothing closed it.
    if (alreadyOpen && NativeXlog.open({ ...config, namePrefix })) {
      return alreadyOpen;
    }
    const xlog = new Xlog(config);
    if (!NativeXlog.open({ ...config, namePrefix })) {
      throw new Error(
        `marsrs-react-native-xlog: the appender of '${xlog.namePrefix}' refused ${config.logDir}`
      );
    }
    openAppenders.set(namePrefix, xlog);
    return xlog;
  }

  /** Whether the appender is still open: `false` after `close`. */
  get isOpen(): boolean {
    return this.open;
  }

  /** The level of this appender: what the appender answers, and not what this
   * instance holds — `mars_xlog_get_level`, which is the one of the five
   * settings the C ABI has a getter for. */
  get level(): LogLevel {
    if (!this.open) {
      return this.currentLevel;
    }
    const level = NativeXlog.getLevel(this.namePrefix) as LogLevel;
    this.currentLevel = level;
    return level;
  }

  set level(level: LogLevel) {
    if (!this.open) {
      return;
    }
    NativeXlog.setLevel(this.namePrefix, level);
    this.currentLevel = level;
  }

  /** The mode of this appender: what `open` or the last write to it gave it,
   * because the C ABI is asked for a mode and never answers one. */
  get mode(): AppenderMode {
    return this.currentMode;
  }

  set mode(mode: AppenderMode) {
    if (!this.open) {
      return;
    }
    NativeXlog.setMode(this.namePrefix, mode);
    this.currentMode = mode;
  }

  /** Whether the console prints the log too: what the last write to it gave it,
   * `false` when there was none. */
  get consoleLogEnabled(): boolean {
    return this.currentConsoleLogEnabled;
  }

  set consoleLogEnabled(enabled: boolean) {
    if (!this.open) {
      return;
    }
    NativeXlog.setConsoleLogEnabled(this.namePrefix, enabled);
    this.currentConsoleLogEnabled = enabled;
  }

  /** The size a log file is split at, in bytes; `0` never splits. */
  get maxFileSizeBytes(): number {
    return this.currentMaxFileSize;
  }

  set maxFileSizeBytes(bytes: number) {
    if (!this.open) {
      return;
    }
    NativeXlog.setMaxFileSize(this.namePrefix, bytes);
    // Mirrored once it is a size the native side took, and not before: that
    // side reads a `number` as an `unsigned long long`, and a negative one or
    // a `NaN` is not one it hands over. `0` is a size and not an absence —
    // "never split" — so it is mirrored like any other.
    if (Number.isFinite(bytes) && bytes >= 0) {
      this.currentMaxFileSize = bytes;
    }
  }

  /** How many seconds a log file of this appender is kept before the sweep
   * deletes it — and not how long one is written to, which is what the day's
   * turn and [maxFileSizeBytes] decide: `0` is the lifetime an appender
   * opened with none keeps, the C++'s own ten days, and one below a day is a
   * lifetime the appender refuses, so what this answers is the one it has and
   * not the last one it was asked for. */
  get maxAliveTimeSeconds(): number {
    return this.currentMaxAliveTime;
  }

  set maxAliveTimeSeconds(seconds: number) {
    if (!this.open) {
      return;
    }
    NativeXlog.setMaxAliveTime(this.namePrefix, seconds);
    // Mirrored once it is a lifetime the appender took, and not before: a day
    // is the shortest one it takes, and `0` is below it, so a mirror that took
    // the number would answer a lifetime no appender is writing under.
    if (seconds >= MIN_ALIVE_TIME_SECONDS) {
      this.currentMaxAliveTime = seconds;
    }
  }

  /** Whether a record at `level` is written: the appender's own answer, and the
   * one the Swift's `isLoggable` and the Kotlin's give. An app asks it before it
   * builds a message that is expensive to build. */
  isLoggable(level: LogLevel): boolean {
    if (!this.open) {
      return false;
    }
    return NativeXlog.isLoggable(this.namePrefix, level);
  }

  /** `mars_xlog_write_instance`: writes `message` at `level`, tagged `tag`.
   *
   * The file, the function and the line of the record are left empty on the
   * platform side — there is no JS frame to name, and the C++ writes an empty
   * one too. */
  log(level: LogLevel, tag: string, message: string): void {
    if (!this.open) {
      return;
    }
    NativeXlog.log(this.namePrefix, level, tag, message);
  }

  /** `log` at `LogLevel.verbose`. */
  v(tag: string, message: string): void {
    this.log(LogLevel.verbose, tag, message);
  }

  /** `log` at `LogLevel.debug`. */
  d(tag: string, message: string): void {
    this.log(LogLevel.debug, tag, message);
  }

  /** `log` at `LogLevel.info`. */
  i(tag: string, message: string): void {
    this.log(LogLevel.info, tag, message);
  }

  /** `log` at `LogLevel.warning`. */
  w(tag: string, message: string): void {
    this.log(LogLevel.warning, tag, message);
  }

  /** `log` at `LogLevel.error`. */
  e(tag: string, message: string): void {
    this.log(LogLevel.error, tag, message);
  }

  /** `log` at `LogLevel.fatal`. */
  f(tag: string, message: string): void {
    this.log(LogLevel.fatal, tag, message);
  }

  /** `mars_xlog_request_flush_instance`: tells the writer thread it may take what is in
   * the cache to the file, and returns at once. Nothing waits, and nothing is
   * in the file because this returned — a record still in the cache is in a file
   * the kernel holds, so one whose process dies keeps it. What it is for is a
   * drain an app wants soon and does not want to wait for; before the file is
   * read, uploaded or left behind, it is `flushNow()` or `await flush()`
   * that an app wants and not this. */
  requestFlush(): void {
    if (!this.open) {
      return;
    }
    NativeXlog.requestFlush(this.namePrefix);
  }

  /** `mars_xlog_flush_now_instance`: the drain is the calling thread's, so the
   * records are in the log file — handed to the OS, and not left in the file's
   * own `FILE*` — when it returns, and what that costs is the time the drain
   * takes, on the thread that asked for it. It answers no `Promise` and takes
   * no `await`, which is the point of it: the line after the call is a line
   * that runs after the drain, and there is nothing to get wrong in between. */
  flushNow(): void {
    if (!this.open) {
      return;
    }
    NativeXlog.flushNow(this.namePrefix);
  }

  /** `flushNow()` off the JS thread: the same drain, on a thread of the
   * module's own, and a `Promise` that is settled when it is over. The records
   * are on disk when it resolves, and what waited for them is that thread and
   * not the one the app runs JS on — the call to make before reading or
   * uploading a log file whose drain is longer than the app can sit through. */
  async flush(): Promise<void> {
    if (!this.open) {
      return;
    }
    await NativeXlog.flush(this.namePrefix);
  }

  /** `mars_xlog_current_log_path_instance`: the directory this appender writes
   * its files into, or `undefined` once it is closed.
   *
   * A directory and not a file, because that is what the C++'s
   * `GetCurrentLogPath` answers, and there is no "not yet" state: an open
   * appender has a directory from the moment it is opened. What an app that
   * uploads a whole day asks for is [`logFiles`]. */
  get currentLogPath(): string | undefined {
    if (!this.open) {
      return undefined;
    }
    return NativeXlog.currentLogPath(this.namePrefix);
  }

  /** `mars_xlog_getfilepath_from_timespan_instance`: the log files of `daysAgo`
   * days ago that are *there* — what an app that uploads yesterday's opens. `[]`
   * when the directory holds none of that day's. `0` is today, `1` is yesterday,
   * and so on.
   *
   * This is a day of files and not the directory they are in: what
   * `currentLogPath` answers is that, and this is this appender's own prefix and
   * directory. */
  logFiles(daysAgo: number): string[] {
    if (!this.open) {
      return [];
    }
    return NativeXlog.logFiles(this.namePrefix, daysAgo);
  }

  /** `mars_xlog_make_logfile_name_instance`: the paths of the log files of
   * `daysAgo` days ago whether or not they are *there yet* — the name an app
   * that is about to write, or that is naming a file to someone else, asks for.
   *
   * A day's answer can be two where [`logFiles`] answers one, when a cache dir
   * is given and the file is there. */
  logFileNames(daysAgo: number): string[] {
    if (!this.open) {
      return [];
    }
    return NativeXlog.logFileNames(this.namePrefix, daysAgo);
  }

  /** `mars_xlog_release_instance`: closes the appender `Xlog.open` made.
   * Nothing is closed twice: an `Xlog` that is already closed answers `false`
   * from `isOpen` and drops what it is asked to write — and an appender two
   * names hold is closed for both, because `Xlog.open` gave them one `Xlog`. */
  close(): void {
    if (!this.open) {
      return;
    }
    this.open = false;
    openAppenders.delete(this.namePrefix);
    NativeXlog.close(this.namePrefix);
  }
}

export default Xlog;
