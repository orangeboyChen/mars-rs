/**
 * `TCompressMode`: how a log file is compressed once it is closed.
 *
 * The numbers are the C ABI's `MarsCompressMode`.
 */
export enum CompressMode {
  /** zlib, what the C++ opens with and what every reader of a mars log file
   * understands. */
  Zlib = 0,
  /** Zstandard: smaller at the same speed, and unreadable by a tool that only
   * knows zlib. */
  Zstd = 1,
}
