// The TypeScript face of the xlog half of mars-rs.
//
// What the two platforms are asked for is the same instance API the Swift and
// the Kotlin of the port call — `mars_xlog_new_instance` and friends over the C
// ABI on iOS, `Xlog.newXlogInstance` over the AAR on Android — so the surface
// below is the one `MarsXlogInstance` of `Sources/MarsRSXlog/Xlog.swift` has:
// one appender, opened with a configuration and closed by the name it was
// opened with.
//
// The fields and their defaults are `XLogConfig`'s; two of them have none —
// `logDirectory`, because the appender answers `MARS_XLOG_ERR_EMPTY_LOG_DIR`
// without it, and `namePrefix`, because `mars_xlog_new_instance` answers `0` —
// a refused configuration — without it. The rest are filled in by the native
// side, so a field left out here is a field the C ABI defaults and not a field
// sent as `undefined`.

import { NativeModules } from 'react-native';

/** `TLogLevel`; `none` is `MARS_LEVEL_NONE`, which the filter understands but
 * the C enum does not carry. */
export const MarsXlogLevel = {
  verbose: 0,
  debug: 1,
  info: 2,
  warning: 3,
  error: 4,
  fatal: 5,
  none: 6,
} as const;

export type MarsXlogLevel = (typeof MarsXlogLevel)[keyof typeof MarsXlogLevel];

/** `TAppenderMode`: whether the appender writes on its own thread or on the
 * caller's. */
export const MarsXlogMode = {
  async: 0,
  sync: 1,
} as const;

export type MarsXlogMode = (typeof MarsXlogMode)[keyof typeof MarsXlogMode];

/** `TCompressMode`. */
export const MarsXlogCompression = {
  zlib: 0,
  zstd: 1,
} as const;

export type MarsXlogCompression =
  (typeof MarsXlogCompression)[keyof typeof MarsXlogCompression];

/** `XLogConfig`, with the defaults the C++ gives the fields it is not told. */
export interface MarsXlogConfig {
  /** Where the log files go: one of the two fields with no default. */
  logDirectory: string;
  /** The level the appender is opened at; `info` when left out. */
  level?: MarsXlogLevel;
  /** Whether the appender writes on its own thread or on the caller's. */
  mode?: MarsXlogMode;
  /** Written verbatim, as the C++ does — and required, because
   * `mars_xlog_new_instance` answers `0` (a refused configuration) for the
   * empty one. */
  namePrefix: string;
  /** Empty means the log is written unencrypted. */
  publicKey?: string;
  /** How the log is compressed; `zlib` is what the C++ defaults to. */
  compression?: MarsXlogCompression;
  /** `0` keeps the appender's own default (6). */
  compressionLevel?: number;
  /** Left out, the mmap cache lives in the log directory. */
  cacheDirectory?: string;
  /** `0` keeps every file. */
  cacheDays?: number;
}

/** The native module, as `MarsRsXlogModule` (Android) and `MarsRsXlog` (iOS)
 * answer it. Every method is a promise, and every promise rejects with the
 * platform's own error. */
interface MarsRsXlogNative {
  open(config: MarsXlogConfig): Promise<void>;
  write(level: number, tag: string, message: string): Promise<void>;
  flush(sync: boolean): Promise<void>;
  setLevel(level: number): Promise<void>;
  setConsoleLog(enabled: boolean): Promise<void>;
  close(): Promise<void>;
}

const nativeModule = NativeModules.MarsRsXlog as MarsRsXlogNative | undefined;

function native(): MarsRsXlogNative {
  if (!nativeModule) {
    throw new Error(
      'mars-rs-react-native-xlog: MarsRsXlog is not linked. Run `pod install` ' +
        'for iOS, and rebuild the app for Android — the module is autolinked.'
    );
  }
  return nativeModule;
}

/** The xlog appender: one instance, opened by `open` and closed by `close`. */
export const MarsRsXlog = {
  /** `mars_xlog_new_instance`: opens an appender of its own with `config`.
   *
   * Rejects when the appender refused the configuration, or when
   * `config.logDirectory` or `config.namePrefix` is empty. */
  open(config: MarsXlogConfig): Promise<void> {
    return native().open(config);
  },

  /** `mars_xlog_write_instance`: writes `message` at `level`, tagged `tag`.
   *
   * The file, the function and the line of the record are the C++'s own
   * defaults on the platform side — there is no JS frame to name. */
  write(level: MarsXlogLevel, message: string, tag: string = ''): Promise<void> {
    return native().write(level, tag, message);
  },

  /** `mars_xlog_flush_instance`; `sync` waits for the write to finish. */
  flush(sync: boolean = false): Promise<void> {
    return native().flush(sync);
  },

  /** `mars_xlog_set_level_instance`. */
  setLevel(level: MarsXlogLevel): Promise<void> {
    return native().setLevel(level);
  },

  /** `mars_xlog_set_console_log_instance`: whether the console prints the log
   * too. */
  setConsoleLogEnabled(enabled: boolean): Promise<void> {
    return native().setConsoleLog(enabled);
  },

  /** `mars_xlog_release_instance`: closes the appender `open` made. */
  close(): Promise<void> {
    return native().close();
  },
};
