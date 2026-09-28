#import "XlogPlugin.h"

#import "mars_xlog.h"

// The iOS half of the `marsrs_flutter_xlog` plugin: the C ABI of `mars_xlog.h`
// (crate `marsrs-ffi`) behind the eleven methods of the plugin's channel.
//
// Objective-C, and not Swift: what the plugin carries is a static library with
// a C header, and `MarsRSFFI` — the module SwiftPM makes of the two — is not
// something CocoaPods can hand a pod, so the glue is written against the header
// itself. It is the instance API every other platform of the port is written
// against: `mars_xlog_new_instance` and friends, one appender per prefix.
//
// `marsrs_flutter` is the whole-port plugin, and this file is its Objective-C
// with the two names changed: xlog is the whole C ABI today, so the two plugins
// are one package under two names.
//
// Every key below is a field `lib/marsrs_flutter_xlog.dart` put there, and every
// default is the one `mars_xlog.h` documents — `logDir` is the field with
// none, because the appender answers `MARS_XLOG_ERR_EMPTY_LOG_DIR` without it.

static NSString *const kXlogChannel = @"marsrs_flutter_xlog";

/// What every `FlutterError` below answers as its code.
static NSString *const kXlogError = @"marsrs_flutter_xlog";

/// `XlogConfig.namePrefix` of the Dart, of the Kotlin and of the Swift: what an
/// appender is opened with when the caller gave none.
static NSString *const kXlogDefaultNamePrefix = @"xlog";

/// The number sent for `key`, or `fallback` when none was sent. A Dart `null`
/// arrives as `NSNull`, which answers no `intValue`.
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
  } else if ([call.method isEqualToString:@"setLevel"]) {
    [self setLevel:call result:result];
  } else if ([call.method isEqualToString:@"getLevel"]) {
    [self getLevel:call result:result];
  } else if ([call.method isEqualToString:@"setMode"]) {
    [self setMode:call result:result];
  } else if ([call.method isEqualToString:@"setConsoleLogEnabled"]) {
    [self setConsoleLogEnabled:call result:result];
  } else if ([call.method isEqualToString:@"setMaxFileSize"]) {
    [self setMaxFileSize:call result:result];
  } else if ([call.method isEqualToString:@"setMaxAliveTime"]) {
    [self setMaxAliveTime:call result:result];
  } else if ([call.method isEqualToString:@"close"]) {
    [self close:call result:result];
  } else {
    result(FlutterMethodNotImplemented);
  }
}

#pragma mark - The channel's methods

/// `mars_xlog_new_instance`. `0` is the answer for a configuration the appender
/// refused, and an instance the caller has no handle to is one every write
/// would go to the process-wide appender instead of.
- (void)open:(FlutterMethodCall *)call result:(FlutterResult)result {
  NSDictionary *arguments = call.arguments;
  NSString *logDir = XlogString(arguments, @"logDir");
  if (logDir.length == 0) {
    result([FlutterError errorWithCode:kXlogError message:@"logDir is empty" details:nil]);
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
    result([FlutterError errorWithCode:kXlogError
                               message:@"the appender refused the configuration"
                               details:nil]);
    return;
  }
  self.instances[namePrefix] = @(handle);
  result(nil);
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
  result(nil);
}

/// `mars_xlog_is_enabled_for`: whether a record of the level would be written,
/// which an app asks before it builds a message that is expensive to build.
- (void)isLoggable:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  int level = XlogInt(call.arguments, @"level", MarsLevelInfo);
  result(@(mars_xlog_is_enabled_for(instance, level) != 0));
}

/// `mars_xlog_flush_instance`.
- (void)flush:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  mars_xlog_flush_instance(instance, XlogInt(call.arguments, @"sync", 0));
  result(nil);
}

/// `mars_xlog_set_level_instance`.
- (void)setLevel:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  mars_xlog_set_level_instance(instance, XlogInt(call.arguments, @"level", MarsLevelInfo));
  result(nil);
}

/// `mars_xlog_get_level`: what the appender answers, and not what Dart holds.
- (void)getLevel:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  result(@(mars_xlog_get_level(instance)));
}

/// `mars_xlog_set_mode_instance`.
- (void)setMode:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  mars_xlog_set_mode_instance(instance, XlogInt(call.arguments, @"mode", MarsAppenderAsync));
  result(nil);
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
  result(nil);
}

/// `mars_xlog_set_max_file_size_instance`.
- (void)setMaxFileSize:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  unsigned long long bytes = XlogUnsignedLong(call.arguments, @"bytes", 0);
  mars_xlog_set_max_file_size_instance(instance, bytes);
  result(nil);
}

/// `mars_xlog_set_max_alive_duration_instance`.
- (void)setMaxAliveTime:(FlutterMethodCall *)call result:(FlutterResult)result {
  long long instance = [self instanceForCall:call result:result];
  if (instance == 0) {
    return;
  }
  mars_xlog_set_max_alive_duration_instance(instance, XlogLong(call.arguments, @"seconds", 0));
  result(nil);
}

/// `mars_xlog_release_instance`: closes the appender `open` made.
- (void)close:(FlutterMethodCall *)call result:(FlutterResult)result {
  NSString *namePrefix = XlogString(call.arguments, @"namePrefix");
  if (self.instances[namePrefix] == nil) {
    result(nil);
    return;
  }
  mars_xlog_release_instance(namePrefix.UTF8String);
  [self.instances removeObjectForKey:namePrefix];
  result(nil);
}

#pragma mark - The appender of a call

/// The handle of the appender the prefix of `call` names, or `0` with `result`
/// answered: no handle is the process-wide appender to the C ABI, so a call that
/// went on with one would write through whatever appender the rest of the
/// process writes through.
- (long long)instanceForCall:(FlutterMethodCall *)call result:(FlutterResult)result {
  NSString *namePrefix = XlogString(call.arguments, @"namePrefix");
  NSNumber *handle = self.instances[namePrefix];
  if (handle == nil) {
    result([FlutterError errorWithCode:kXlogError
                               message:[NSString stringWithFormat:
                                                     @"no appender of '%@' is open", namePrefix]
                               details:nil]);
    return 0;
  }
  return handle.longLongValue;
}

@end
