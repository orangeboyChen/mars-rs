// What `libmarsrs_xlog.so` exports: the NAPI methods `src/main/ets/xlog/Xlog.ets`
// calls, and the only way an ArkTS file sees the native half.
//
// This is a declaration and not an implementation — `src/main/cpp/napi_init.cpp`
// is the other half of this file, and the two are kept apart by nothing but the
// names in it, which is why the names are the C ABI's own (`open`, `log`,
// `requestFlush`, `close`) and the parameter order is the order of the C functions
// behind them.
//
// Every method is synchronous and answers no promise: a NAPI method that
// answers no promise is a call, which is what lets `Xlog` of the ArkTS be
// shaped like the Kotlin and not like the Flutter.
//
// A number, and not an enum, is what crosses: `LogLevel`, `AppenderMode` and
// `CompressMode` are ArkTS enums over the C ABI's own integers, so a level is
// spelled `LogLevel.Info` in the app and handed over as `2`.

export interface XlogConfig {
  logDir: string;
  namePrefix?: string;
  level?: number;
  mode?: number;
  pubKey?: string;
  compressMode?: number;
  compressLevel?: number;
  cacheDir?: string;
  cacheDays?: number;
}

/** `mars_xlog_new_instance`: opens the appender of `config`. `false` when the
 * appender would not take it — an empty `logDir`, or a directory the app cannot
 * write to. */
export const open: (config: XlogConfig) => boolean;

/** `mars_xlog_get_level`: the appender's level, or the one it was opened at. */
export const getLevel: (namePrefix: string) => number;

/** `mars_xlog_set_level_instance`. */
export const setLevel: (namePrefix: string, level: number) => void;

/** `mars_xlog_set_mode_instance`. */
export const setMode: (namePrefix: string, mode: number) => void;

/** `mars_xlog_set_console_log_instance`. */
export const setConsoleLogEnabled: (namePrefix: string, enabled: boolean) => void;

/** `mars_xlog_set_max_file_size_instance`; `0` never splits. */
export const setMaxFileSize: (namePrefix: string, bytes: number) => void;

/** `mars_xlog_set_max_alive_duration_instance`; `0` is ten days. */
export const setMaxAliveTime: (namePrefix: string, seconds: number) => void;

/** `mars_xlog_is_enabled_for`: whether a record at `level` would be written. */
export const isLoggable: (namePrefix: string, level: number) => boolean;

/** `mars_xlog_write_instance`: writes `message` at `level`, tagged `tag`. */
export const log: (namePrefix: string, level: number, tag: string, message: string) => void;

/** `mars_xlog_request_flush_instance`: tells the writer thread it may take what
 * is in the cache to the file, and returns at once. Nothing is guaranteed to
 * have been written when it does. */
export const requestFlush: (namePrefix: string) => void;

/** `mars_xlog_flush_now_instance`: takes what is in the cache to the file on
 * the calling thread, so the records are on disk when it returns. It answers
 * no promise — the wait is the call. */
export const flushNow: (namePrefix: string) => void;

/** `mars_xlog_current_log_path_instance`: the file the appender of `namePrefix`
 * is writing to, or `undefined` when it has none open yet. */
export const currentLogPath: (namePrefix: string) => string | undefined;

/** `mars_xlog_getfilepath_from_timespan_instance`: the log files of `daysAgo`
 * days ago that are *there* — `0` is today, `1` is yesterday. */
export const logFiles: (namePrefix: string, daysAgo: number) => string[];

/** `mars_xlog_make_logfile_name_instance`: the paths of the log files of
 * `daysAgo` days ago, whether or not they are there yet. */
export const logFileNames: (namePrefix: string, daysAgo: number) => string[];

/** `mars_xlog_release_instance`: closes the appender `open` made. */
export const close: (namePrefix: string) => void;
