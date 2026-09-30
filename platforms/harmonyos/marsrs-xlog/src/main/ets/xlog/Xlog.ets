import xlogNapi from 'libmarsrs_xlog.so';

import { AppenderMode } from './AppenderMode';
import { LogLevel } from './LogLevel';
import { XlogConfig } from './XlogConfig';

// The ArkTS face of the xlog half of mars-rs.
//
// This `Xlog` is the `Xlog` of `platforms/apple/MarsRSXlog/Xlog.swift`, of
// `platforms/android/marsrs-xlog`, of `platforms/kmp/marsrs-xlog` and of
// `platforms/react-native/marsrs-xlog` member for member: `Xlog.open(config)`
// answers the appender, `xlog.level` is the property it is in Kotlin and in
// Swift, and `xlog.i(tag, message)` has landed by the time it returns.
//
// Nothing here awaits. A NAPI method that returns no promise is a call and not
// an `await`, which is why the shape is the Kotlin one and not the Flutter one:
// the record is handed to the C ABI in the call, and the call is over when it
// returns.
//
// The one member the Swift's `Xlog` has and this one does not is `flush()`:
// an awaited drain is an async work item and a promise, and every method of the
// NAPI module behind this file is synchronous — so a caller that wants the drain
// off its own thread gives `flushNow()` a thread of its own, and one that wants
// it off the writer's and off no thread of the app's calls `requestFlush()`.
//
// Five settings are properties and not `setLevel` / `getLevel` pairs, because a
// property is the Kotlin and the Swift spelling. `xlog.level` reads the
// appender's own — `mars_xlog_get_level`, the one of the five the C ABI has a
// getter for; `mode`, `consoleLogEnabled`, `maxFileSizeBytes` and
// `maxAliveTimeSeconds` answer what this instance last wrote, because neither
// the C ABI, nor the Kotlin, nor the Swift answers one.

/** `XlogConfig.namePrefix` of the Kotlin and of the Swift: what an appender is
 * opened with when a caller gives none. */
const DEFAULT_NAME_PREFIX: string = 'xlog';

/** The appender of every `namePrefix` `Xlog.open` has opened and `close` has not
 * closed, by the prefix: what makes two `Xlog.open`s of one prefix one appender,
 * which the native side already is — one module, one appender per prefix, and
 * every call carrying the prefix it is about.
 *
 * The second `open` answers the appender the first made, so the two names an app
 * holds are one `Xlog`: `close` on either is `close` on both, and nothing writes
 * through an appender that is gone — which is what it would do, and in silence,
 * if the second call were a second instance the first one's `close` did not know
 * about. */
const openAppenders: Map<string, Xlog> = new Map<string, Xlog>();

/** The appender of one `namePrefix`: the `Xlog` of the Swift and the Kotlin of
 * the port. `Xlog.open` makes it, and `close` releases the appender it made. */
export class Xlog {
  /** The name the appender is known by, and what every one of its files starts
   * with: the `namePrefix` of the Kotlin and of the Swift. */
  readonly namePrefix: string;

  private currentLevel: LogLevel = LogLevel.Info;

  private currentMode: AppenderMode = AppenderMode.Async;

  private currentConsoleLogEnabled: boolean = false;

  private currentMaxFileSize: number = 0;

  private currentMaxAliveTime: number = 0;

  /** Whether `close` has been called on this instance. */
  private opened: boolean = true;

  private constructor(config: XlogConfig) {
    this.namePrefix = config.namePrefix ?? DEFAULT_NAME_PREFIX;
    this.currentLevel = config.level ?? LogLevel.Info;
    this.currentMode = config.mode ?? AppenderMode.Async;
  }

  /** `mars_xlog_new_instance`: opens the appender of `config` and answers the
   * `Xlog` that writes through it.
   *
   * The appender of a `namePrefix` this has already opened is answered as it is,
   * and not opened again: the native side is one appender per prefix, and a
   * second one over the first would be a handle nothing releases. So two calls
   * of one prefix are one `Xlog`, and `close` on it is `close` on both.
   *
   * Throws when the appender would not take the configuration — an empty
   * `logDir`, or a directory the app cannot write to. */
  static open(config: XlogConfig): Xlog {
    let namePrefix: string = config.namePrefix ?? DEFAULT_NAME_PREFIX;
    let alreadyOpen: Xlog | undefined = openAppenders.get(namePrefix);
    if (alreadyOpen !== undefined) {
      return alreadyOpen;
    }
    let xlog: Xlog = new Xlog(config);
    if (!xlogNapi.open(config)) {
      throw new Error(
        `marsrs-harmonyos-xlog: the appender of '${xlog.namePrefix}' refused ${config.logDir}`
      );
    }
    openAppenders.set(namePrefix, xlog);
    return xlog;
  }

  /** Whether the appender is still open: `false` after `close`. */
  get isOpen(): boolean {
    return this.opened;
  }

  /** The level of this appender: what the appender answers, and not what this
   * instance holds — `mars_xlog_get_level`, which is the one of the five
   * settings the C ABI has a getter for. */
  get level(): LogLevel {
    if (!this.opened) {
      return this.currentLevel;
    }
    let level: LogLevel = xlogNapi.getLevel(this.namePrefix) as LogLevel;
    this.currentLevel = level;
    return level;
  }

  set level(level: LogLevel) {
    if (!this.opened) {
      return;
    }
    xlogNapi.setLevel(this.namePrefix, level);
    this.currentLevel = level;
  }

  /** The mode of this appender: what `open` or the last write to it gave it,
   * because the C ABI is asked for a mode and never answers one. */
  get mode(): AppenderMode {
    return this.currentMode;
  }

  set mode(mode: AppenderMode) {
    if (!this.opened) {
      return;
    }
    xlogNapi.setMode(this.namePrefix, mode);
    this.currentMode = mode;
  }

  /** Whether the console prints the log too: what the last write to it gave it,
   * `false` when there was none. */
  get consoleLogEnabled(): boolean {
    return this.currentConsoleLogEnabled;
  }

  set consoleLogEnabled(enabled: boolean) {
    if (!this.opened) {
      return;
    }
    xlogNapi.setConsoleLogEnabled(this.namePrefix, enabled);
    this.currentConsoleLogEnabled = enabled;
  }

  /** The size a log file is split at, in bytes; `0` never splits. */
  get maxFileSizeBytes(): number {
    return this.currentMaxFileSize;
  }

  set maxFileSizeBytes(bytes: number) {
    if (!this.opened) {
      return;
    }
    xlogNapi.setMaxFileSize(this.namePrefix, bytes);
    this.currentMaxFileSize = bytes;
  }

  /** How long a log file is written to before the appender opens the next one,
   * in seconds; `0` is the C++'s own ten days. */
  get maxAliveTimeSeconds(): number {
    return this.currentMaxAliveTime;
  }

  set maxAliveTimeSeconds(seconds: number) {
    if (!this.opened) {
      return;
    }
    xlogNapi.setMaxAliveTime(this.namePrefix, seconds);
    this.currentMaxAliveTime = seconds;
  }

  /** `mars_xlog_current_log_path_instance`: the *directory* this appender
   * writes its files into, or `undefined` once it is closed.
   *
   * A directory and not a file, because that is what the C++ answers
   * (`GetCurrentLogPath` hands back `sg_logdir`); the day's file is what
   * [`logFiles`] names. There is no "before the first record" state — `open`
   * answers a directory from the moment it succeeds. */
  get currentLogPath(): string | undefined {
    if (!this.opened) {
      return undefined;
    }
    return xlogNapi.currentLogPath(this.namePrefix);
  }

  /** Whether a record at `level` is written: the appender's own answer, and the
   * one the Swift's `isLoggable` and the Kotlin's give. An app asks it before it
   * builds a message that is expensive to build. */
  isLoggable(level: LogLevel): boolean {
    if (!this.opened) {
      return false;
    }
    return xlogNapi.isLoggable(this.namePrefix, level);
  }

  /** `mars_xlog_write_instance`: writes `message` at `level`, tagged `tag`.
   *
   * The file, the function and the line of the record are left empty on the
   * native side — there is no ArkTS frame worth naming, and the C++ writes an
   * empty one too. */
  log(level: LogLevel, tag: string, message: string): void {
    if (!this.opened) {
      return;
    }
    xlogNapi.log(this.namePrefix, level, tag, message);
  }

  /** `log` at `LogLevel.Verbose`. */
  v(tag: string, message: string): void {
    this.log(LogLevel.Verbose, tag, message);
  }

  /** `log` at `LogLevel.Debug`. */
  d(tag: string, message: string): void {
    this.log(LogLevel.Debug, tag, message);
  }

  /** `log` at `LogLevel.Info`. */
  i(tag: string, message: string): void {
    this.log(LogLevel.Info, tag, message);
  }

  /** `log` at `LogLevel.Warning`. */
  w(tag: string, message: string): void {
    this.log(LogLevel.Warning, tag, message);
  }

  /** `log` at `LogLevel.Error`. */
  e(tag: string, message: string): void {
    this.log(LogLevel.Error, tag, message);
  }

  /** `log` at `LogLevel.Fatal`. */
  f(tag: string, message: string): void {
    this.log(LogLevel.Fatal, tag, message);
  }

  /** `mars_xlog_request_flush_instance`: tells the writer thread it may take
   * what is in the cache to the log file, and returns at once.
   *
   * It guarantees nothing. The drain is the writer thread's, it has not
   * necessarily begun when this returns, and a record that is still in the
   * cache is not in the file yet — so a caller that reads the day's files or
   * uploads them wants `flushNow()` and not this. What this is for is a drain
   * an app wants soon and does not want to wait for: what is in the cache is
   * in a file the kernel holds, so a drain that has not happened yet has lost
   * nothing. */
  requestFlush(): void {
    if (!this.opened) {
      return;
    }
    xlogNapi.requestFlush(this.namePrefix);
  }

  /** `mars_xlog_flush_now_instance`: takes what is in the cache to the
   * log file on the calling thread, and hands the file's own buffer to the OS
   * — the last few KiB of a log file sit in a `FILE*` until this runs, so a
   * reader in another process cannot see them yet.
   *
   * The records are on disk when it returns, and what it costs is the time the
   * drain takes, on the thread that asked for it. It answers no promise and
   * takes no callback, because a drain that is over when the call returns is
   * not something to await: blocking a thread is the cost, and it is the
   * caller's to pay or not. */
  flushNow(): void {
    if (!this.opened) {
      return;
    }
    xlogNapi.flushNow(this.namePrefix);
  }

  /**
   * `mars_xlog_getfilepath_from_timespan_instance`: the log files of `daysAgo`
   * days ago that are *there* — what an app that uploads yesterday's opens.
   * `[]` when the directory holds none of that day's. `0` is today, `1` is
   * yesterday, and so on.
   *
   * This is a day of files and not the file being written: what
   * `currentLogPath` answers is one, and this is *this* appender's own prefix
   * and directory.
   */
  logFiles(daysAgo: number): string[] {
    if (!this.opened) {
      return [];
    }
    return xlogNapi.logFiles(this.namePrefix, daysAgo);
  }

  /**
   * `mars_xlog_make_logfile_name_instance`: the paths of the log files of
   * `daysAgo` days ago whether or not they are *there yet* — the name an app
   * that is about to write, or that is naming a file to someone else, asks
   * for. A day's answer can be two where `logFiles` answers one, when a cache
   * dir is given and the file is there.
   */
  logFileNames(daysAgo: number): string[] {
    if (!this.opened) {
      return [];
    }
    return xlogNapi.logFileNames(this.namePrefix, daysAgo);
  }

  /** `mars_xlog_release_instance`: closes the appender `Xlog.open` made.
   * Nothing is closed twice: an `Xlog` that is already closed answers `false`
   * from `isOpen` and drops what it is asked to write — and an appender two
   * names hold is closed for both, because `Xlog.open` gave them one `Xlog`. */
  close(): void {
    if (!this.opened) {
      return;
    }
    this.opened = false;
    openAppenders.delete(this.namePrefix);
    xlogNapi.close(this.namePrefix);
  }
}
