// The TurboModule spec of `Xlog`: the file codegen reads, and what
// `src/index.ts` calls at runtime.
//
// Codegen makes two things of it — `NativeXlogSpec`, the abstract class the
// Android half extends, and `NativeXlogSpec`, the protocol the iOS half conforms
// to — and the two of them are what let a call land where it is made instead of
// crossing a bridge: a TurboModule method that answers no promise is called on
// the JS thread and returns, and one that answers a value answers it before it
// returns. That is the only reason `src/index.ts` can be the Kotlin and the
// Swift member for member — `Xlog.open(config)`, `xlog.level`,
// `xlog.i(tag, message)` — where the bridge's eleven `Promise`s left it with a
// `setLevel`, a `getLevel` and an `await` in front of every line.
//
// The configuration crosses as an `Object` and not as a declared type: codegen
// reads this file and not the ones it imports, so `XlogConfig` is documented in
// `src/index.ts`, where an app reads it — and a `ReadableMap` and an
// `NSDictionary` is what the two halves take either way.

import type { TurboModule } from 'react-native';
import { TurboModuleRegistry } from 'react-native';

export interface Spec extends TurboModule {
  /** `Xlog.open`: whether the appender took the configuration. `false` is a
   * configuration it refused, which is an empty `logDir` or `namePrefix`, or a
   * directory it cannot write to. */
  open(config: Object): boolean;

  /** `mars_xlog_write_instance`. The file, the function and the line are left
   * empty on the platform side: there is no JS frame to name, and the C++ writes
   * an empty one too. */
  log(namePrefix: string, level: number, tag: string, message: string): void;

  /** `mars_xlog_is_enabled_for`: whether a record of the level would be written. */
  isLoggable(namePrefix: string, level: number): boolean;

  /** `mars_xlog_get_level`: the level the appender is at, and not the one JS
   * holds. */
  getLevel(namePrefix: string): number;

  /** `mars_xlog_request_flush_instance`: tells the writer thread it may take what is in
   * the cache to the file, and returns at once — nothing waits, and nothing is
   * in the file because this returned. */
  requestFlush(namePrefix: string): void;

  /** `mars_xlog_flush_now_instance`: the drain is the calling thread's, so the cache
   * is in the log file, and the file's buffer is the OS's, when this returns. */
  flushNow(namePrefix: string): void;

  /** `flushNow` on a thread of the module's own, and a `Promise` settled when
   * the drain is over: the one of the three that leaves the JS thread alone.
   * A `Promise<void>` and not a `Promise` of a value — there is nothing to
   * answer, only a drain to be over. */
  flush(namePrefix: string): Promise<void>;

  /** `mars_xlog_current_log_path_instance`: the directory this appender writes
   * its files into, or `undefined` once it is closed. */
  currentLogPath(namePrefix: string): string | undefined;

  /** `mars_xlog_getfilepath_from_timespan_instance`: the day's files that are
   * there — `0` is today, `1` is yesterday. */
  logFiles(namePrefix: string, daysAgo: number): string[];

  /** `mars_xlog_make_logfile_name_instance`: the day's names, whether or not the
   * files are there yet. */
  logFileNames(namePrefix: string, daysAgo: number): string[];

  /** `mars_xlog_set_level_instance`. */
  setLevel(namePrefix: string, level: number): void;

  /** `mars_xlog_set_mode_instance`. */
  setMode(namePrefix: string, mode: number): void;

  /** `mars_xlog_set_console_log_instance`. */
  setConsoleLogEnabled(namePrefix: string, enabled: boolean): void;

  /** `mars_xlog_set_max_file_size_instance`. A `number` and not an `Int64`: the
   * module carries every JS number as one, and a file size is below 2^53. */
  setMaxFileSize(namePrefix: string, bytes: number): void;

  /** `mars_xlog_set_max_alive_duration_instance`. */
  setMaxAliveTime(namePrefix: string, seconds: number): void;

  /** Releases the appender `open` made: by handle on iOS, by prefix on
   * Android. */
  close(namePrefix: string): void;
}

export default TurboModuleRegistry.getEnforcing<Spec>('Xlog');
