#import "XlogPlugin.h"

#import "mars_xlog.h"

// The iOS half of the `marsrs_xlog` plugin: the C ABI of `mars_xlog.h`
// (crate `marsrs-ffi`) behind the fifteen methods of the plugin's channel.
//
// Objective-C, and not Swift: what the plugin carries is a static library with
// a C header, and `MarsRSFFI` — the module SwiftPM makes of the two — is not
// something CocoaPods can hand a pod, so the glue is written against the header
// itself. It is the instance API every other platform of the port is written
// against: `mars_xlog_new_instance` and friends, one appender per prefix.
//
// `marsrs` is the whole-port plugin, and this file is its Objective-C
// with the two names changed: xlog is the whole C ABI today, so the two plugins
// are one package under two names.
//
// Every key below is a field `lib/marsrs_xlog.dart` put there, and every
// default is the one `mars_xlog.h` documents — `logDir` is the field with
// none, because the C ABI opens no appender without it.

static NSString *const kXlogChannel = @"marsrs_xlog";

/// What every `FlutterError` below answers as its code.
static NSString *const kXlogError = @"marsrs_xlog";

/// `XlogConfig.namePrefix` of the Dart, of the Kotlin and of the Swift: what an
/// appender is opened with when the caller gave none.
static NSString *const kXlogDefaultNamePrefix = @"xlog";

/// Answers `result` with `value`. `FlutterResult` is a block and not a message
/// send, so a nil one is a null function pointer and not a no-op: every answer
/// on this side goes through here.
static void XlogAnswer(FlutterResult result, id value) {
  if (result != nil) {
    result(value);
  }
}

/// What `read` writes into the buffer it is handed, as a string; `nil` when it
/// wrote nothing — a negative code, or a path of no length.
///
/// A negative code is `nil` whatever it is, `MARS_XLOG_ERR_NO_SPACE` among
/// them: a path that does not fit 1024 bytes ends the walk the way the end of
/// the list does. A symbol that answers a length rather than a pointer is the
/// C ABI's way of saying the caller decides how much it can hold.
static NSString *XlogPath(int (^read)(char *, uint32_t)) {
  char buffer[1024];
  int written = read(buffer, (uint32_t)sizeof(buffer));
  if (written <= 0) {
    return nil;
  }
  return [[NSString alloc] initWithBytes:buffer
                                  length:(NSUInteger)written
                                encoding:NSUTF8StringEncoding];
}

/// The number sent for `key`, or `fallback` when none was sent — what a level,
/// a mode and a cache day are. A Dart `null` arrives as `NSNull`, which
/// answers no `intValue`.
static int XlogInt(NSDictionary *arguments, NSString *key, int fallback) {
  NSNumber *value = arguments[key];
  return [value isKindOfClass:NSNumber.class] ? value.intValue : fallback;
}

/// The number sent for `key` as a `long long`, which is what an alive duration
/// is.
static long long XlogLong(NSDictionary *arguments, NSString *key, long long fallback) {
  NSNumber *value = arguments[key];
  return [value isKindOfClass:NSNumber.class] ? value.longLongValue : fallback;
}

/// The number sent for `key` as an `unsigned long long`, which is what a file
/// size is.
static unsigned long long XlogUnsignedLong(NSDictionary *arguments,
                                           NSString *key,
                                           unsigned long long fallback) {
  NSNumber *value = arguments[key];
  return [value isKindOfClass:NSNumber.class] ? value.unsignedLongLongValue : fallback;
}

/// The string sent for `key`, or the empty one.
static NSString *XlogString(NSDictionary *arguments, NSString *key) {
  NSString *value = arguments[key];
  return [value isKindOfClass:NSString.class] ? value : @"";
}

/// The string sent for `key`, or `nil` — `cacheDir` is the one field a
/// caller may leave out, and `NULL` is what `mars_xlog.h` reads as "empty".
static NSString *XlogOptionalString(NSDictionary *arguments, NSString *key) {
  NSString *value = arguments[key];
  return [value isKindOfClass:NSString.class] && value.length > 0 ? value : nil;
}

@interface XlogPlugin ()

/// The appender of every prefix `open` has opened, by the prefix: what keeps
/// two `Xlog`s apart, which one handle cannot, because the appender a
/// `mars_xlog_release_instance` releases is the one of the prefix it is given.
@property(nonatomic, strong) NSMutableDictionary<NSString *, NSNumber *> *instances;

@end

@implementation XlogPlugin

+ (void)registerWithRegistrar:(NSObject<FlutterPluginRegistrar> *)registrar {
  FlutterMethodChannel *channel =
      [FlutterMethodChannel methodChannelWithName:kXlogChannel
                                  binaryMessenger:registrar.messenger];
  XlogPlugin *plugin = [[XlogPlugin alloc] init];
  [registrar addMethodCallDelegate:plugin channel:channel];
}

- (instancetype)init {
  self = [super init];
  if (self) {
    _instances = [[NSMutableDictionary alloc] init];
  }
  return self;
}

- (void)handleMethodCall:(FlutterMethodCall *)call result:(FlutterResult)result {
  if ([call.method isEqualToString:@"open"]) {
    [self open:call result:result];
  } else if ([call.method isEqualToString:@"log"]) {
    [self log:call result:result];
  } else if ([call.method isEqualToString:@"isLoggable"]) {
    [self isLoggable:call result:result];
  } else if ([call.method isEqualToString:@"flush"]) {
    [self flush:call result:result];
  } else if ([call.method isEqualToString:@"requestFlush"]) {
    [self requestFlush:call result:result];
  } else if ([call.method isEqualToString:@"setLevel"]) {
    [self setLevel:call result:result];
  } else if ([call.method isEqualToString:@"setMode"]) {
    [self setMode:call result:result];
  } else if ([call.method isEqualToString:@"setConsoleLogEnabled"]) {
    [self setConsoleLogEnabled:call result:result];
  } else if ([call.method isEqualToString:@"setMaxFileSize"]) {
    [self setMaxFileSize:call result:result];
  } else if ([call.method isEqualToString:@"setMaxAliveTime"]) {
    [self setMaxAliveTime:call result:result];
  } else if ([call.method isEqualToString:@"currentLogPath"]) {
    [self currentLogPath:call result:result];
  } else if ([call.method isEqualToString:@"logFiles"]) {
    [self logFiles:call result:result];
  } else if ([call.method isEqualToString:@"logFileNames"]) {
    [self logFileNames:call result:result];
  } else if ([call.method isEqualToString:@"close"]) {
    [self close:call result:result];
  } else {
    XlogAnswer(result, FlutterMethodNotImplemented);
  }
}

#pragma mark - The channel's methods

/// `mars_xlog_new_instance`. `0` is the answer for a configuration the appender
/// refused, and a handle nothing was opened for is one every write through it
/// would silently drop.
- (void)open:(FlutterMethodCall *)call result:(FlutterResult)result {
  NSDictionary *arguments = call.arguments;
  NSString *logDir = XlogString(arguments, @"logDir");
  if (logDir.length == 0) {
    XlogAnswer(result, [FlutterError errorWithCode:kXlogError message:@"logDir is empty" details:nil]);
    return;
  }
  NSString *namePrefix = XlogString(arguments, @"namePrefix");
  if (namePrefix.length == 0) {
    namePrefix = kXlogDefaultNamePrefix;
  }

  MarsXLogConfig config;
  config.mode = XlogInt(arguments, @"mode", MarsAppenderAsync);
  config.log_dir = logDir.UTF8String;
  config.name_prefix = namePrefix.UTF8String;
  config.pub_key = XlogString(arguments, @"pubKey").UTF8String;
  config.compress_mode = XlogInt(arguments, @"compressMode", MarsCompressZlib);
  config.compress_level = XlogInt(arguments, @"compressLevel", 0);
  config.cache_dir = XlogOptionalString(arguments, @"cacheDir").UTF8String;
  config.cache_days = XlogInt(arguments, @"cacheDays", 0);

  long long handle =
      mars_xlog_new_instance(&config, XlogInt(arguments, @"level", MarsLevelInfo));
  if (handle == 0) {
    XlogAnswer(result, [FlutterError errorWithCode:kXlogError
                               message:@"the appender refused the configuration"
                               details:nil]);
    return;
  }
  self.instances[namePrefix] = @(handle);
  XlogAnswer(result, nil);
}

/// `mars_xlog_current_log_path_instance` — the directory this appender writes
/// its files into.
- (void)currentLogPath:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  XlogAnswer(result, XlogPath(^(char *out, uint32_t len) {
    return mars_xlog_current_log_path_instance(instance, out, len);
  }));
}

/// `mars_xlog_getfilepath_from_timespan_instance` — the day's files that are
/// there.
- (void)logFiles:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  XlogAnswer(result, [self dayPathsOfInstance:instance
                         daysAgo:XlogInt(call.arguments, @"daysAgo", 0)
                            with:^int(long long instance, int timespan, unsigned int index, char *out,
                                      unsigned int len) {
                              return mars_xlog_getfilepath_from_timespan_instance(instance, timespan, index, out, len);
                            }]);
}

/// `mars_xlog_make_logfile_name_instance` — the day's names, whether or not the
/// files are there yet.
- (void)logFileNames:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  XlogAnswer(result, [self dayPathsOfInstance:instance
                         daysAgo:XlogInt(call.arguments, @"daysAgo", 0)
                            with:^int(long long instance, int timespan, unsigned int index, char *out,
                                      unsigned int len) {
                              return mars_xlog_make_logfile_name_instance(instance, timespan, index, out, len);
                            }]);
}

/// A day of paths, walked index by index until the symbol answers that there is
/// nothing at that index — the list the C++ fills a `std::vector` with, asked
/// one at a time.
- (NSArray<NSString *> *)dayPathsOfInstance:(long long)instance
                                    daysAgo:(int)timespan
                                       with:(int (^)(long long, int, unsigned int, char *, unsigned int))pathAt {
  NSMutableArray<NSString *> *walked = [NSMutableArray array];
  for (unsigned int index = 0;; index++) {
    NSString *found = XlogPath(^(char *out, uint32_t len) {
      return pathAt(instance, timespan, index, out, len);
    });
    if (found == nil) {
      break;
    }
    [walked addObject:found];
  }
  return walked;
}

/// `mars_xlog_write_instance`. The file, the function and the line are left
/// empty: there is no Dart frame to name, and the C++ writes an empty one too.
- (void)log:(FlutterMethodCall *)call result:(FlutterResult)result {
  NSDictionary *arguments = call.arguments;
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  mars_xlog_write_instance(instance,
                           XlogInt(arguments, @"level", MarsLevelInfo),
                           XlogString(arguments, @"tag").UTF8String,
                           "",
                           "",
                           0,
                           XlogString(arguments, @"message").UTF8String);
  XlogAnswer(result, nil);
}

/// `mars_xlog_is_enabled_for`: whether a record of the level would be written,
/// which an app asks before it builds a message that is expensive to build.
- (void)isLoggable:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  int level = XlogInt(call.arguments, @"level", MarsLevelInfo);
  XlogAnswer(result, @(mars_xlog_is_enabled_for(instance, level) != 0));
}

/// `mars_xlog_flush_now_instance`: the drain is on the thread this is called
/// on, which is what the Dart caller awaiting `flush()` is waiting for, and
/// what the writer thread that `requestFlush:` only wakes is not.
- (void)flush:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  // Off the thread the call came in on, and back to it for the answer: a
  // platform channel is answered on the app's main thread, and a drain blocks
  // the thread it runs on.
  dispatch_async(dispatch_get_global_queue(DISPATCH_QUEUE_PRIORITY_DEFAULT, 0), ^{
    mars_xlog_flush_now_instance(instance);
    dispatch_async(dispatch_get_main_queue(), ^{
      XlogAnswer(result, nil);
    });
  });
}

/// `mars_xlog_request_flush_instance`: the drain is the writer thread's.
- (void)requestFlush:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  mars_xlog_request_flush_instance(instance);
  XlogAnswer(result, nil);
}

/// `mars_xlog_set_level_instance`.
- (void)setLevel:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  mars_xlog_set_level_instance(instance, XlogInt(call.arguments, @"level", MarsLevelInfo));
  XlogAnswer(result, nil);
}


/// `mars_xlog_set_mode_instance`.
- (void)setMode:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  mars_xlog_set_mode_instance(instance, XlogInt(call.arguments, @"mode", MarsAppenderAsync));
  XlogAnswer(result, nil);
}

/// `mars_xlog_set_console_log_instance`.
- (void)setConsoleLogEnabled:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  NSNumber *enabled = call.arguments[@"enabled"];
  BOOL on = [enabled isKindOfClass:NSNumber.class] ? enabled.boolValue : NO;
  mars_xlog_set_console_log_instance(instance, on ? 1 : 0);
  XlogAnswer(result, nil);
}

/// `mars_xlog_set_max_file_size_instance`.
- (void)setMaxFileSize:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  unsigned long long bytes = XlogUnsignedLong(call.arguments, @"bytes", 0);
  mars_xlog_set_max_file_size_instance(instance, bytes);
  XlogAnswer(result, nil);
}

/// `mars_xlog_set_max_alive_duration_instance`.
- (void)setMaxAliveTime:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  mars_xlog_set_max_alive_duration_instance(instance, XlogLong(call.arguments, @"seconds", 0));
  XlogAnswer(result, nil);
}

/// `mars_xlog_release_instance_of`: closes the appender `open` made.
- (void)close:(FlutterMethodCall *)call result:(FlutterResult)result {
  NSString *namePrefix = XlogString(call.arguments, @"namePrefix");
  NSNumber *handle = self.instances[namePrefix];
  if (handle == nil) {
    XlogAnswer(result, nil);
    return;
  }
  long long opened = handle.longLongValue;
  [self.instances removeObjectForKey:namePrefix];
  // `mars_xlog_release_instance_of` and not `mars_xlog_release_instance`: a
  // release is given a prefix, so what it closes is whichever appender the
  // prefix answers *at that moment* — one another part of the app opened
  // after this one was closed. Naming the handle makes the question and the
  // release one call, which a `mars_xlog_get_instance` before a release is
  // not: a `close` on another thread lands between the two.
  mars_xlog_release_instance_of(namePrefix.UTF8String, opened);
  XlogAnswer(result, nil);
}

#pragma mark - The appender of a call

/// The handle of the appender the prefix of `call` names, or `0` with `result`
/// answered: a handle whose appender is gone is a no-op to the C ABI, so a call
/// that went on with one would silently write nothing.
- (long long)instanceForCall:(FlutterMethodCall *)call result:(FlutterResult)result {
  NSString *namePrefix = XlogString(call.arguments, @"namePrefix");
  NSNumber *handle = self.instances[namePrefix];
  // The registry and not the handle [instances] holds: `mars_xlog_release_instance`
  // releases the appender of a *prefix* and not of a handle, so a handle whose
  // appender another part of the app closed is still the number this dictionary
  // holds — and every symbol of the C ABI answers nothing for a handle it does
  // not know, which is a call that silently writes nothing.
  long long opened = handle == nil ? 0 : handle.longLongValue;
  if (opened == 0 || mars_xlog_get_instance(namePrefix.UTF8String) != opened) {
    XlogAnswer(result,
               [FlutterError errorWithCode:kXlogError
                                   message:[NSString stringWithFormat:
                                                         @"no appender of '%@' is open", namePrefix]
                                   details:nil]);
    return 0;
  }
  return opened;
}

@end
