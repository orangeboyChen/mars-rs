// The Dart face of the xlog half of mars-rs.
//
// A method channel, and not `dart:ffi`: the Apple binary is a static library
// inside `MarsRSXlog.xcframework`, and `DynamicLibrary.open` has nothing to open
// for one. So what the two halves call is the instance API the Swift and the
// Kotlin of the port publish — `mars_xlog_new_instance` and friends over the C
// ABI on iOS, `Xlog(XlogConfig(...))` over the AAR on Android.
//
// Every call crosses the channel, so every one of them is a `Future`: the write
// is an `await` and not a call the caller can let go. That is the one thing this
// surface cannot have in common with the Swift and the Kotlin — there, a record
// costs a call and nothing else — and it is why the settings below are
// `setLevel` and not `level`.

import 'package:flutter/services.dart';

/// `TLogLevel`; `.none` is `MARS_LEVEL_NONE`, which the filter understands but
/// the C enum does not carry.
enum LogLevel {
  verbose(0),
  debug(1),
  info(2),
  warning(3),
  error(4),
  fatal(5),
  none(6);

  const LogLevel(this.value);

  /// The number `mars_xlog.h` gives the level.
  final int value;
}

/// `TAppenderMode`: whether the appender writes on its own thread or on the
/// caller's.
enum AppenderMode {
  async(0),
  sync(1);

  const AppenderMode(this.value);

  /// The number `mars_xlog.h` gives the mode.
  final int value;
}

/// `TCompressMode`.
enum CompressMode {
  zlib(0),
  zstd(1);

  const CompressMode(this.value);

  /// The number `mars_xlog.h` gives the compression.
  final int value;
}

/// `XLogConfig` of the C++ project, in the spelling the Kotlin of the port
/// gives it: what an `Xlog` is opened with.
class XlogConfig {
  const XlogConfig({
    required this.logDir,
    this.namePrefix = 'xlog',
    this.level = LogLevel.info,
    this.mode = AppenderMode.async,
    this.pubKey = '',
    this.compressMode = CompressMode.zlib,
    this.compressLevel = 0,
    this.cacheDir,
    this.cacheDays = 0,
  });

  /// Where the log files go: the one field with no default.
  final String logDir;

  /// What every file starts with, and the name the appender is known by.
  final String namePrefix;

  /// The level a record has to reach.
  final LogLevel level;

  /// Whether a write waits for the file.
  final AppenderMode mode;

  /// Empty writes the log unencrypted.
  final String pubKey;

  /// How a closed file is compressed.
  final CompressMode compressMode;

  /// `0` keeps the compressor's own default.
  final int compressLevel;

  /// `null` puts the mmap cache next to the log files.
  final String? cacheDir;

  /// `0` keeps every file.
  final int cacheDays;

  /// The configuration as the method channel carries it.
  Map<String, Object?> toMap() {
    return <String, Object?>{
      'logDir': logDir,
      'namePrefix': namePrefix,
      'level': level.value,
      'mode': mode.value,
      'pubKey': pubKey,
      'compressMode': compressMode.value,
      'compressLevel': compressLevel,
      'cacheDir': cacheDir,
      'cacheDays': cacheDays,
    };
  }
}

/// One appender: opened by [Xlog.open] and closed by [Xlog.close].
class Xlog {
  Xlog._(this.namePrefix);

  static const MethodChannel _channel = MethodChannel('marsrs_flutter_xlog');

  /// What every file of this appender starts with, and what it is known by.
  final String namePrefix;

  var _closed = false;

  /// Whether this appender is still open: `false` after [close].
  bool get isOpen => !_closed;

  /// Opens an appender of its own with [config].
  ///
  /// The platform side answers `marsrs_flutter_xlog` / `the appender refused the
  /// configuration` when the appender would not take it, and
  /// `logDir is empty` when [XlogConfig.logDir] is.
  static Future<Xlog> open(XlogConfig config) async {
    await _channel.invokeMethod<void>('open', config.toMap());
    return Xlog._(config.namePrefix);
  }

  /// Every call carries the name the appender was opened with, because the
  /// native side is one plugin and not an object per appender: it is what lets
  /// an app write through two of them — two prefixes, two appenders — the way
  /// it writes through two `Xlog`s in Kotlin and in Swift.
  Future<T?> _invoke<T>(String method, [Map<String, Object?> args = const <String, Object?>{}]) {
    return _channel.invokeMethod<T>(method, <String, Object?>{'namePrefix': namePrefix, ...args});
  }

  /// Writes one record of [level], tagged [tag].
  ///
  /// The file, the function and the line of the record are the C++'s own
  /// defaults on the platform side — Dart has no caller frame to name.
  Future<void> log(LogLevel level, String tag, String message) {
    return _invoke<void>('log', <String, Object?>{'level': level.value, 'tag': tag, 'message': message});
  }

  /// [LogLevel.verbose].
  Future<void> v(String tag, String message) => log(LogLevel.verbose, tag, message);

  /// [LogLevel.debug].
  Future<void> d(String tag, String message) => log(LogLevel.debug, tag, message);

  /// [LogLevel.info].
  Future<void> i(String tag, String message) => log(LogLevel.info, tag, message);

  /// [LogLevel.warning].
  Future<void> w(String tag, String message) => log(LogLevel.warning, tag, message);

  /// [LogLevel.error].
  Future<void> e(String tag, String message) => log(LogLevel.error, tag, message);

  /// [LogLevel.fatal].
  Future<void> f(String tag, String message) => log(LogLevel.fatal, tag, message);

  /// Whether a record of [level] would be written: what an app asks before it
  /// builds a message that is expensive to build.
  Future<bool> isLoggable(LogLevel level) async {
    return await _invoke<bool>('isLoggable', <String, Object?>{'level': level.value}) ?? false;
  }

  /// Takes what is in the cache to the log file; [sync] waits for the write.
  Future<void> flush({bool sync = false}) {
    return _invoke<void>('flush', <String, Object?>{'sync': sync});
  }

  /// The level a record has to reach.
  Future<void> setLevel(LogLevel level) {
    return _invoke<void>('setLevel', <String, Object?>{'level': level.value});
  }

  /// The level the appender is at: what [setLevel] last asked for, and what the
  /// configuration gave before that.
  Future<LogLevel> getLevel() async {
    final value = await _invoke<int>('getLevel');
    return LogLevel.values[(value ?? LogLevel.info.value).clamp(0, LogLevel.values.length - 1)];
  }

  /// Whether a write waits for the file.
  Future<void> setMode(AppenderMode mode) {
    return _invoke<void>('setMode', <String, Object?>{'mode': mode.value});
  }

  /// Whether the console prints the record too.
  Future<void> setConsoleLogEnabled(bool enabled) {
    return _invoke<void>('setConsoleLogEnabled', <String, Object?>{'enabled': enabled});
  }

  /// How many bytes a log file may reach before it is closed and a new one
  /// opened; `0` never splits.
  Future<void> setMaxFileSize(int bytes) {
    return _invoke<void>('setMaxFileSize', <String, Object?>{'bytes': bytes});
  }

  /// How many seconds a log file is kept; `0` is the C++'s own ten days.
  Future<void> setMaxAliveTime(int seconds) {
    return _invoke<void>('setMaxAliveTime', <String, Object?>{'seconds': seconds});
  }

  /// Drains what is left and closes this appender. Writing through it afterwards
  /// writes nothing.
  Future<void> close() async {
    await _invoke<void>('close');
    _closed = true;
  }
}
