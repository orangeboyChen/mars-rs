// What `libmarsrs_xlog.so` exports: the NAPI methods `src/main/ets/xlog/Xlog.ets`
// calls, and the only way an ArkTS file sees the native half.
//
// This is a declaration and not an implementation — `src/main/cpp/napi_init.cpp`
// is the other half of this file, and the two are kept apart by nothing but the
// names in it, which is why the names are the C ABI's own (`open`, `log`,
// `flush`, `close`) and the parameter order is the order of the C functions
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

/** `mars_xlog_flush_instance`; `sync` waits for the write. */
export const flush: (namePrefix: string, sync: boolean) => void;

/** `mars_xlog_release_instance`: closes the appender `open` made. */
export const close: (namePrefix: string) => void;
