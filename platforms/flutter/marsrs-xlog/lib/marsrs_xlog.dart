// The Dart face of the xlog half of mars-rs.
//
// A method channel, and not `dart:ffi`: the Apple binary is a static library
// inside `MarsRSXlog.xcframework`, and `DynamicLibrary.open` has nothing to open
// for one. So what the two halves call is the instance API the Swift and the
// Kotlin of the port publish — `mars_xlog_new_instance` and friends over the C
// ABI on iOS, `Xlog(XlogConfig(...))` over the AAR on Android.
//
// A call that answers nothing waits for nothing: a write and a setting hand the
// call to the platform side and return, `xlog.i('net', '…')` the way it does in
// Kotlin and in Swift, and the channel keeps the order the calls were handed
// over in. Four of them answer a `Future`, because four of them have something
// an app can act on: the appender [Xlog.open] opens, the drain [Xlog.flush] and
// [Xlog.close] wait for, and the answer [Xlog.isLoggable] gives.
//
// The five settings are properties and not a `setLevel` / `getLevel` pair, which
// is the spelling the Kotlin, the Swift and the TypeScript of the port give
// them. What one of them answers is what this side last wrote, and not what the
// appender holds: a getter answers in the call it is read in, and what the
// platform side holds is a channel call away. [Xlog.isLoggable] asks the
// appender itself, which is why it is the one of the six an app awaits.

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
  Xlog._(this.namePrefix, this._level, this._mode);

  static const MethodChannel _channel = MethodChannel('marsrs_xlog');

  /// What every file of this appender starts with, and what it is known by.
  final String namePrefix;

  /// The drain [close] started, and `null` while it has not: what a second
  /// [close] answers, so that every caller waits for the one drain, and what
  /// marks this appender closed while that drain is still running — a record
  /// handed to an appender that is closing is one with nowhere to land.
  Future<void>? _closing;

  LogLevel _level;

  AppenderMode _mode;

  var _consoleLogEnabled = false;

  var _maxFileSizeBytes = 0;

  var _maxAliveTimeSeconds = 0;

  /// Whether this appender is still open: `false` once [close] is called, and
  /// not only once the drain it started has finished.
  bool get isOpen => _closing == null;

  /// Opens an appender of its own with [config].
  ///
  /// The platform side answers `marsrs_xlog` / `the appender refused the
  /// configuration` when the appender would not take it, and
  /// `logDir is empty` when [XlogConfig.logDir] is.
  static Future<Xlog> open(XlogConfig config) async {
    await _channel.invokeMethod<void>('open', config.toMap());
    return Xlog._(config.namePrefix, config.level, config.mode);
  }

  /// The level a record has to reach: what this was last set to, and what the
  /// configuration gave before that.
  ///
  /// The level the appender itself is at is what [isLoggable] reads, and it is
  /// the one an app that did not set it asks for: a getter answers in the call
  /// it is read in, and a level the platform side holds is a channel call away.
  LogLevel get level => _level;

  set level(LogLevel level) {
    if (_closing != null) {
      return;
    }
    _level = level;
    _send('setLevel', <String, Object?>{'level': level.value});
  }

  /// Whether a write waits for the file: what the configuration gave, and what
  /// this was last set to after that.
  ///
  /// The C ABI is asked for a mode and never answers one, so a mode is a mirror
  /// on every platform of the port and not only here.
  AppenderMode get mode => _mode;

  set mode(AppenderMode mode) {
    if (_closing != null) {
      return;
    }
    _mode = mode;
    _send('setMode', <String, Object?>{'mode': mode.value});
  }

  /// Whether the console prints the record too: what this was last set to, and
  /// `false` when it never was.
  bool get consoleLogEnabled => _consoleLogEnabled;

  set consoleLogEnabled(bool enabled) {
    if (_closing != null) {
      return;
    }
    _consoleLogEnabled = enabled;
    _send('setConsoleLogEnabled', <String, Object?>{'enabled': enabled});
  }

  /// How many bytes a log file may reach before it is closed and a new one
  /// opened; `0` never splits.
  int get maxFileSizeBytes => _maxFileSizeBytes;

  set maxFileSizeBytes(int bytes) {
    if (_closing != null) {
      return;
    }
    _maxFileSizeBytes = bytes;
    _send('setMaxFileSize', <String, Object?>{'bytes': bytes});
  }

  /// How many seconds a log file is kept; `0` is the C++'s own ten days.
  int get maxAliveTimeSeconds => _maxAliveTimeSeconds;

  set maxAliveTimeSeconds(int seconds) {
    if (_closing != null) {
      return;
    }
    _maxAliveTimeSeconds = seconds;
    _send('setMaxAliveTime', <String, Object?>{'seconds': seconds});
  }

  /// Whether a record of [level] would be written: what an app asks before it
  /// builds a message that is expensive to build.
  ///
  /// The appender's own answer, and not the one [level] mirrors — and `false`
  /// once [close] ran, which is the honest answer of an appender that writes
  /// nothing.
  Future<bool> isLoggable(LogLevel level) async {
    if (_closing != null) {
      return false;
    }
    return await _invoke<bool>('isLoggable', <String, Object?>{'level': level.value}) ?? false;
  }

  /// Writes one record of [level], tagged [tag].
  ///
  /// The file, the function and the line of the record are the C++'s own
  /// defaults on the platform side — Dart has no caller frame to name.
  void log(LogLevel level, String tag, String message) {
    if (_closing != null) {
      return;
    }
    _send('log', <String, Object?>{'level': level.value, 'tag': tag, 'message': message});
  }

  /// [LogLevel.verbose].
  void v(String tag, String message) => log(LogLevel.verbose, tag, message);

  /// [LogLevel.debug].
  void d(String tag, String message) => log(LogLevel.debug, tag, message);

  /// [LogLevel.info].
  void i(String tag, String message) => log(LogLevel.info, tag, message);

  /// [LogLevel.warning].
  void w(String tag, String message) => log(LogLevel.warning, tag, message);

  /// [LogLevel.error].
  void e(String tag, String message) => log(LogLevel.error, tag, message);

  /// [LogLevel.fatal].
  void f(String tag, String message) => log(LogLevel.fatal, tag, message);

  /// Takes what is in the cache to the log file; [sync] waits for the write.
  ///
  /// Waiting is the platform side's: what this answers is that the drain has
  /// happened, which is what an app wants before it reads or uploads the files.
  Future<void> flush({bool sync = false}) async {
    if (_closing != null) {
      return;
    }
    await _invoke<void>('flush', <String, Object?>{'sync': sync});
  }

  /// Drains what is left and closes this appender. Writing through it afterwards
  /// writes nothing, and calling it twice closes nothing twice: what the second
  /// call answers is the drain the first one started, so an app that awaits
  /// either one has awaited the one drain, and not a future that was already
  /// done while the files were still being written — the answer an app waiting
  /// to read or upload them would have been given by a second call that
  /// returned at once.
  Future<void> close() async {
    final closing = _closing;
    if (closing != null) {
      return closing;
    }
    final drain = _invoke<void>('close');
    _closing = drain;
    return drain;
  }

  /// Every call carries the name the appender was opened with, because the
  /// native side is one plugin and not an object per appender: it is what lets
  /// an app write through two of them — two prefixes, two appenders — the way
  /// it writes through two `Xlog`s in Kotlin and in Swift.
  Future<T?> _invoke<T>(String method, [Map<String, Object?> args = const <String, Object?>{}]) {
    return _channel.invokeMethod<T>(method, <String, Object?>{'namePrefix': namePrefix, ...args});
  }

  /// Hands `method` to the platform side and does not wait for it: a write and
  /// a setting have nothing to answer, and the channel keeps the order the
  /// calls were made in, so a record handed over is a record written after the
  /// one handed over before it.
  ///
  /// What the platform side answers is ignored, an error included — a write
  /// through an appender that is not open is one the platform side refuses, and
  /// a logger that throws at the app that logged is worse than a record that is
  /// not in the file.
  void _send(String method, [Map<String, Object?> args = const <String, Object?>{}]) {
    _invoke<void>(method, args).ignore();
  }
}
