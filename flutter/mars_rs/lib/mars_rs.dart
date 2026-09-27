// The Dart face of mars-rs: the whole port, which today is xlog. The plugin
// this file is in is `mars_rs`, and `mars_rs_xlog` is the same plugin with the
// logging half only — two names for one package until STN and SDT land in the
// C ABI, and they land here.
//
// A method channel, and not `dart:ffi`: the Apple binary is a static library
// inside `MarsRSXlog.xcframework`, and `DynamicLibrary.open` has nothing to
// open for one. So what the two halves call is the same instance API the Swift
// and the Kotlin of the port call — `mars_xlog_new_instance` and friends over
// the C ABI on iOS, `Xlog.newXlogInstance` over the AAR on Android — and the
// surface below is the one `MarsXlogInstance` of
// `Sources/MarsRSXlog/Xlog.swift` has: an appender of its own, opened with a
// configuration and released by the name it was opened with.
//
// The fields and their defaults are `XLogConfig`'s; `logDirectory` is the one
// with no default, because the appender answers
// `MARS_XLOG_ERR_EMPTY_LOG_DIR` without it.

import 'package:flutter/services.dart';

/// `TLogLevel`; `.none` is `MARS_LEVEL_NONE`, which the filter understands but
/// the C enum does not carry.
enum MarsXlogLevel {
  verbose(0),
  debug(1),
  info(2),
  warning(3),
  error(4),
  fatal(5),
  none(6);

  const MarsXlogLevel(this.value);

  /// The number `mars_xlog.h` gives the level.
  final int value;
}

/// `TAppenderMode`: whether the appender writes on its own thread or on the
/// caller's.
enum MarsXlogMode {
  async(0),
  sync(1);

  const MarsXlogMode(this.value);

  /// The number `mars_xlog.h` gives the mode.
  final int value;
}

/// `TCompressMode`.
enum MarsXlogCompression {
  zlib(0),
  zstd(1);

  const MarsXlogCompression(this.value);

  /// The number `mars_xlog.h` gives the compression.
  final int value;
}

/// `XLogConfig`, with the defaults the C++ gives the fields it is not told.
class MarsXlogConfig {
  const MarsXlogConfig({
    required this.logDirectory,
    this.level = MarsXlogLevel.info,
    this.mode = MarsXlogMode.async,
    this.namePrefix = '',
    this.publicKey = '',
    this.compression = MarsXlogCompression.zlib,
    this.compressionLevel = 0,
    this.cacheDirectory,
    this.cacheDays = 0,
  });

  /// The level the appender is opened at.
  final MarsXlogLevel level;

  /// Whether the appender writes on its own thread or on the caller's.
  final MarsXlogMode mode;

  /// Where the log files go: the one field with no default.
  final String logDirectory;

  /// Written verbatim, as the C++ does: no default.
  final String namePrefix;

  /// Empty means the log is written unencrypted.
  final String publicKey;

  /// How the log is compressed; `.zlib` is what the C++ defaults to.
  final MarsXlogCompression compression;

  /// `0` keeps the appender's own default (6).
  final int compressionLevel;

  /// `null` puts the mmap cache in the log directory.
  final String? cacheDirectory;

  /// `0` keeps every file.
  final int cacheDays;

  /// The configuration as the method channel carries it.
  Map<String, Object?> toMap() {
    return <String, Object?>{
      'level': level.value,
      'mode': mode.value,
      'logDirectory': logDirectory,
      'namePrefix': namePrefix,
      'publicKey': publicKey,
      'compression': compression.value,
      'compressionLevel': compressionLevel,
      'cacheDirectory': cacheDirectory,
      'cacheDays': cacheDays,
    };
  }
}

/// The xlog appender of this plugin: one instance, opened by [MarsRsXlog.open]
/// and released by [MarsRsXlog.close].
class MarsRsXlog {
  MarsRsXlog._();

  static const MethodChannel _channel = MethodChannel('mars_rs_xlog');

  /// `mars_xlog_new_instance`: opens an appender of its own with [config].
  ///
  /// The platform side answers `mars_rs_xlog` / `the appender refused the
  /// configuration` when the appender would not take it, and
  /// `logDirectory is empty` when [MarsXlogConfig.logDirectory] is.
  static Future<void> open(MarsXlogConfig config) {
    return _channel.invokeMethod<void>('open', config.toMap());
  }

  /// `mars_xlog_write_instance`: writes [message] at [level], tagged [tag].
  ///
  /// The file, the function and the line of the record are the C++'s own
  /// defaults on the platform side — Dart has no caller frame to name.
  static Future<void> write(
    MarsXlogLevel level,
    String message, {
    String tag = '',
  }) {
    return _channel.invokeMethod<void>('write', <String, Object?>{
      'level': level.value,
      'tag': tag,
      'message': message,
    });
  }

  /// `mars_xlog_flush_instance`; [sync] waits for the write to finish.
  static Future<void> flush({bool sync = false}) {
    return _channel.invokeMethod<void>('flush', <String, Object?>{'sync': sync});
  }

  /// `mars_xlog_set_level_instance`.
  static Future<void> setLevel(MarsXlogLevel level) {
    return _channel.invokeMethod<void>('setLevel', <String, Object?>{
      'level': level.value,
    });
  }

  /// `mars_xlog_set_console_log_instance`: whether the console prints the log
  /// too.
  static Future<void> setConsoleLogEnabled(bool enabled) {
    return _channel.invokeMethod<void>('setConsoleLog', <String, Object?>{
      'enabled': enabled,
    });
  }

  /// `mars_xlog_release_instance`: closes the appender [open] made.
  static Future<void> close() {
    return _channel.invokeMethod<void>('close');
  }
}
