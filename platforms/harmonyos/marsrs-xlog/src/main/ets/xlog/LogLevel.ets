/**
 * `TLogLevel` of the C++ project, in the spelling the Kotlin of the port gives
 * it. `None` is `MARS_LEVEL_NONE`, which the filter understands and the C enum
 * does not carry: an appender at `None` writes nothing.
 *
 * The numbers are the C ABI's `MarsLogLevel`, and they are what the NAPI module
 * hands `mars_xlog_write_instance` — a level is a number at the boundary and an
 * enum here, so nothing has to be translated twice.
 */
export enum LogLevel {
  Verbose = 0,
  Debug = 1,
  Info = 2,
  Warning = 3,
  Error = 4,
  Fatal = 5,
  /** Understood by the filter only: every record is dropped. */
  None = 6,
}
