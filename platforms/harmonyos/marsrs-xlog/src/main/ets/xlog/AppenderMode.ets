/**
 * `TAppenderMode`: whether a record reaches the file before the call returns.
 *
 * The numbers are the C ABI's `MarsAppenderMode`.
 */
export enum AppenderMode {
  /** The record goes into the memory-mapped cache and a writer thread takes it
   * to the file — what the C++ opens with, and what `flushNow` drains. */
  Async = 0,
  /** The record is in the file before the call returns, at a write and a lock
   * per line. */
  Sync = 1,
}
