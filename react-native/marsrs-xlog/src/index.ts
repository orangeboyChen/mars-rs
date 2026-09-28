// The TypeScript face of the xlog half of mars-rs.
//
// A native module, and not a JSI binding: the Apple binary is a static library
// inside `MarsRSXlog.xcframework`, and there is nothing to `dlopen` for one. So
// what the two halves call is the instance API every other platform of the port
// calls — `mars_xlog_new_instance` and friends over the C ABI on iOS,
// `Xlog(XlogConfig(...))` over the AAR on Android — and what is below is the
// `Xlog` of `Sources/MarsRSXlog/Xlog.swift`, of `android/marsrs-xlog` and of
// `kmp/marsrs-xlog`: the same class, the same configuration, the same six
// helpers, under the same names.
//
// Two things a bridge cannot be, and they are the two places this file is not
// the Kotlin:
//
//   * `Xlog.open` answers a `Promise`, because the appender is opened on the
//     platform's thread — a JS constructor cannot await, so the constructor is
//     `open` and not `new Xlog(...)`.
//   * The settings are methods — `setLevel`, `setMode`, `setMaxFileSize` …
//     — because a JS setter cannot be awaited either. What an app reads back is
//     the value this instance holds: the one `open` or the last `setLevel` gave
//     it, which is the appender's and no other's.
//
// The level, the mode and the two limits are held here too, because a JS
// property cannot be awaited: what `level` answers is what `open` or the last
// `setLevel` gave this instance, and `getLevel` is the call that asks the
// appender for its own.

import { NativeModules } from 'react-native';

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

/** `TAppenderMode`: whether a record reaches the file before the call
 * returns. */
export const AppenderMode = {
  /** The record goes into the memory-mapped cache and a writer thread takes it
   * to the file — what the C++ opens with, and what `flush` drains. */
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
 * app has to give is `logDir` — the appender answers
 * `MARS_XLOG_ERR_EMPTY_LOG_DIR` without it. */
export interface XlogConfig {
  /** Where the log files are written; created if it is missing, and the one
   * field with no default. */
  logDir: string;
  /** What every log file starts with (`marsrs_20260927.xlog`), and the name the
   * appender is known by — an app that writes through two of them gives them
   * two. `xlog` when left out. */
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

/** The native module, as `XlogModule` (Android) and `Xlog` (iOS) answer it.
 * Every method is a promise, and every promise rejects with the platform's own
 * error. Each one carries the prefix of the appender it is about, because that
 * is what an appender is known by on both platforms. */
interface XlogNative {
  open(config: XlogConfig): Promise<void>;
  log(namePrefix: string, level: number, tag: string, message: string): Promise<void>;
  isLoggable(namePrefix: string, level: number): Promise<boolean>;
  getLevel(namePrefix: string): Promise<number>;
  flush(namePrefix: string, sync: boolean): Promise<void>;
  setLevel(namePrefix: string, level: number): Promise<void>;
  setMode(namePrefix: string, mode: number): Promise<void>;
  setConsoleLogEnabled(namePrefix: string, enabled: boolean): Promise<void>;
  setMaxFileSize(namePrefix: string, bytes: number): Promise<void>;
  setMaxAliveTime(namePrefix: string, seconds: number): Promise<void>;
  close(namePrefix: string): Promise<void>;
}

/** `XlogConfig.namePrefix` of the Kotlin and of the Swift: what an appender is
 * opened with when a caller gives none. */
const DEFAULT_NAME_PREFIX = 'xlog';

const nativeModule = NativeModules.Xlog as XlogNative | undefined;

function native(): XlogNative {
  if (!nativeModule) {
    throw new Error(
      'marsrs-react-native-xlog: Xlog is not linked. Run `pod install` for iOS, ' +
        'and rebuild the app for Android — the module is autolinked.'
    );
  }
  return nativeModule;
}

/** The appender of one `namePrefix`: the `Xlog` of the Swift and the Kotlin of
 * the port. `Xlog.open` makes it, and `close` releases the appender it made. */
export class Xlog {
  private readonly config: XlogConfig;

  readonly namePrefix: string;

  private currentLevel: LogLevel;

  private currentMode: AppenderMode;

  private currentConsoleLogEnabled = false;

  private currentMaxFileSize = 0;

  private currentMaxAliveTime = 0;

  /** Whether `close` has been called on this instance. */
  private open = true;

  private constructor(config: XlogConfig) {
    this.config = config;
    this.namePrefix = config.namePrefix ?? DEFAULT_NAME_PREFIX;
    this.currentLevel = config.level ?? LogLevel.info;
    this.currentMode = config.mode ?? AppenderMode.async;
  }

  /** `mars_xlog_new_instance`: opens the appender of `config` and answers the
   * `Xlog` that writes through it.
   *
   * Rejects when the appender would not take the configuration — an empty
   * `logDir` or `namePrefix`, or a directory it cannot write to. */
  static async open(config: XlogConfig): Promise<Xlog> {
    const xlog = new Xlog(config);
    await native().open(config);
    return xlog;
  }

  /** Whether the appender is still open: `false` after `close`. */
  get isOpen(): boolean {
    return this.open;
  }

  /** The level of this appender: the one `open` or the last `setLevel` gave
   * it. */
  get level(): LogLevel {
    return this.currentLevel;
  }

  /** The mode of this appender: the one `open` or the last `setMode` gave it. */
  get mode(): AppenderMode {
    return this.currentMode;
  }

  /** Whether the console prints the log too: what the last
   * `setConsoleLogEnabled` gave it, `false` when it was never called. */
  get consoleLogEnabled(): boolean {
    return this.currentConsoleLogEnabled;
  }

  /** The size a log file is split at, in bytes; `0` never splits. */
  get maxFileSizeBytes(): number {
    return this.currentMaxFileSize;
  }

  /** How long a log file is written to before the appender opens the next one,
   * in seconds; `0` is the C++'s own ten days. */
  get maxAliveTimeSeconds(): number {
    return this.currentMaxAliveTime;
  }

  /** Whether a record at `level` is written: the appender's own answer, and the
   * one the Swift's `isEnabled(for:)` and the Kotlin's `isLoggable` give. An app
   * asks it before it builds a message that is expensive to build. */
  async isLoggable(level: LogLevel): Promise<boolean> {
    if (!this.open) {
      return false;
    }
    return native().isLoggable(this.namePrefix, level);
  }

  /** `mars_xlog_write_instance`: writes `message` at `level`, tagged `tag`.
   *
   * The file, the function and the line of the record are left empty on the
   * platform side — there is no JS frame to name, and the C++ writes an empty
   * one too. */
  async log(level: LogLevel, tag: string, message: string): Promise<void> {
    if (!this.open) {
      return;
    }
    await native().log(this.namePrefix, level, tag, message);
  }

  /** `log` at `LogLevel.verbose`. */
  v(tag: string, message: string): Promise<void> {
    return this.log(LogLevel.verbose, tag, message);
  }

  /** `log` at `LogLevel.debug`. */
  d(tag: string, message: string): Promise<void> {
    return this.log(LogLevel.debug, tag, message);
  }

  /** `log` at `LogLevel.info`. */
  i(tag: string, message: string): Promise<void> {
    return this.log(LogLevel.info, tag, message);
  }

  /** `log` at `LogLevel.warning`. */
  w(tag: string, message: string): Promise<void> {
    return this.log(LogLevel.warning, tag, message);
  }

  /** `log` at `LogLevel.error`. */
  e(tag: string, message: string): Promise<void> {
    return this.log(LogLevel.error, tag, message);
  }

  /** `log` at `LogLevel.fatal`. */
  f(tag: string, message: string): Promise<void> {
    return this.log(LogLevel.fatal, tag, message);
  }

  /** `mars_xlog_flush_instance`: takes what is in the cache to the file. `sync`
   * waits for the write, which is what an app wants before it reads the file,
   * uploads it, or lets the process go. */
  async flush(sync = false): Promise<void> {
    if (!this.open) {
      return;
    }
    await native().flush(this.namePrefix, sync);
  }

  /** `mars_xlog_set_level_instance`: moves the level of this appender to
   * `level`. */
  async setLevel(level: LogLevel): Promise<void> {
    if (!this.open) {
      return;
    }
    await native().setLevel(this.namePrefix, level);
    this.currentLevel = level;
  }

  /** `mars_xlog_get_level`: the level this appender is at — what `open` or the
   * last `setLevel` gave it, read back from the appender and not from this
   * instance. */
  async getLevel(): Promise<LogLevel> {
    if (!this.open) {
      return this.currentLevel;
    }
    const level = await native().getLevel(this.namePrefix);
    this.currentLevel = level;
    return level;
  }

  /** `mars_xlog_set_mode_instance`: moves this appender to `mode`. */
  async setMode(mode: AppenderMode): Promise<void> {
    if (!this.open) {
      return;
    }
    await native().setMode(this.namePrefix, mode);
    this.currentMode = mode;
  }

  /** `mars_xlog_set_console_log_instance`: whether the console prints the log
   * too. */
  async setConsoleLogEnabled(enabled: boolean): Promise<void> {
    if (!this.open) {
      return;
    }
    await native().setConsoleLogEnabled(this.namePrefix, enabled);
    this.currentConsoleLogEnabled = enabled;
  }

  /** `mars_xlog_set_max_file_size_instance`: the size a log file is split at;
   * `0` never splits. */
  async setMaxFileSize(bytes: number): Promise<void> {
    if (!this.open) {
      return;
    }
    await native().setMaxFileSize(this.namePrefix, bytes);
    this.currentMaxFileSize = bytes;
  }

  /** `mars_xlog_set_max_alive_duration_instance`: how long a log file is
   * written to before the appender opens the next one; `0` is the C++'s own ten
   * days. */
  async setMaxAliveTime(seconds: number): Promise<void> {
    if (!this.open) {
      return;
    }
    await native().setMaxAliveTime(this.namePrefix, seconds);
    this.currentMaxAliveTime = seconds;
  }

  /** `mars_xlog_release_instance`: closes the appender `Xlog.open` made.
   * Nothing is closed twice: an `Xlog` that is already closed answers `false`
   * from `isOpen` and drops what it is asked to write. */
  async close(): Promise<void> {
    if (!this.open) {
      return;
    }
    this.open = false;
    await native().close(this.namePrefix);
  }
}

export default Xlog;
