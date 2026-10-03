import { LogLevel } from './LogLevel';
import { AppenderMode } from './AppenderMode';
import { CompressMode } from './CompressMode';

/**
 * `XLogConfig` of the C++ project, in the spelling the Kotlin of the port gives
 * it: what an `Xlog` is opened with.
 *
 * Every field has the default the C++'s own `XLogConfig` carries, so the one an
 * app has to give is `logDir` — the C ABI opens no appender
 * without it.
 */
export interface XlogConfig {
  /** Where the log files are written; created if it is missing, and the one
   * field with no default. */
  logDir: string;

  /** What every log file starts with (`marsrs_20260927.xlog`), and the name the
   * appender is known by — an app that writes through two of them gives them
   * two. `xlog` when left out. */
  namePrefix?: string;

  /** The level the appender is opened at; `Info` when left out. */
  level?: LogLevel;

  /** Whether a write reaches the file before it returns. */
  mode?: AppenderMode;

  /** Empty, or left out, means the log is written unencrypted. */
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
