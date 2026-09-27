#import "MarsRsXlogPlugin.h"

#import "mars_xlog.h"

// The iOS half of the `mars_rs_xlog` plugin: the C ABI of `mars_xlog.h` (crate
// `mars-ffi`) behind the six methods of the plugin's channel.
//
// Objective-C, and not Swift: what the plugin carries is a static library with
// a C header, and `MarsRSFFI` — the module SwiftPM makes of the two — is not
// something CocoaPods can hand a pod, so the glue is written against the header
// itself. It is the same instance API `MarsXlogInstance` of
// `Sources/MarsRSXlog/Xlog.swift` is: `mars_xlog_new_instance` and friends.
//
// Every field below is one `lib/mars_rs_xlog.dart` sent, and every default is
// the one `mars_xlog.h` documents for the field — `log_dir` is the one with no
// default, because the appender answers `MARS_XLOG_ERR_EMPTY_LOG_DIR` without
// it.

static NSString *const kMarsRsXlogChannel = @"mars_rs_xlog";

/// The number sent for `key`, or `fallback` when none was sent. A Dart `null`
/// arrives as `NSNull`, which answers no `intValue`.
static int MarsRsXlogInt(NSDictionary *arguments, NSString *key, int fallback) {
  NSNumber *value = arguments[key];
  return [value isKindOfClass:NSNumber.class] ? value.intValue : fallback;
}

/// The string sent for `key`, or the empty one.
static NSString *MarsRsXlogString(NSDictionary *arguments, NSString *key) {
  NSString *value = arguments[key];
  return [value isKindOfClass:NSString.class] ? value : @"";
}

/// The string sent for `key`, or `nil` — `cacheDirectory` is the one field a
/// caller may leave out, and `NULL` is what `mars_xlog.h` reads as "empty".
static NSString *MarsRsXlogOptionalString(NSDictionary *arguments, NSString *key) {
  NSString *value = arguments[key];
  return [value isKindOfClass:NSString.class] ? value : nil;
}

@interface MarsRsXlogPlugin ()

/// The instance `mars_xlog_new_instance` gave; `0` before `open`, after `close`.
@property(nonatomic) int64_t instance;

/// What `close` releases the instance by: the prefix it was opened with.
@property(nonatomic, copy) NSString *namePrefix;

@end

@implementation MarsRsXlogPlugin

+ (void)registerWithRegistrar:(NSObject<FlutterPluginRegistrar> *)registrar {
  FlutterMethodChannel *channel =
      [FlutterMethodChannel methodChannelWithName:kMarsRsXlogChannel
                                  binaryMessenger:registrar.messenger];
  MarsRsXlogPlugin *plugin = [[MarsRsXlogPlugin alloc] init];
  [registrar addMethodCallDelegate:plugin channel:channel];
}

- (void)handleMethodCall:(FlutterMethodCall *)call result:(FlutterResult)result {
  if ([call.method isEqualToString:@"open"]) {
    [self open:call result:result];
  } else if ([call.method isEqualToString:@"write"]) {
    [self write:call];
    result(nil);
  } else if ([call.method isEqualToString:@"flush"]) {
    [self flush:call];
    result(nil);
  } else if ([call.method isEqualToString:@"setLevel"]) {
    [self setLevel:call];
    result(nil);
  } else if ([call.method isEqualToString:@"setConsoleLog"]) {
    [self setConsoleLog:call];
    result(nil);
  } else if ([call.method isEqualToString:@"close"]) {
    [self close];
    result(nil);
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
  NSString *logDirectory = MarsRsXlogString(arguments, @"logDirectory");
  if (logDirectory.length == 0) {
    result([FlutterError errorWithCode:@"mars_rs_xlog"
                               message:@"logDirectory is empty"
                               details:nil]);
    return;
  }
  NSString *namePrefix = MarsRsXlogString(arguments, @"namePrefix");
  // `mars_xlog_new_instance` answers `0` for the empty prefix, and an appender
  // it refused is an appender the caller has no handle to.
  if (namePrefix.length == 0) {
    result([FlutterError errorWithCode:@"mars_rs_xlog"
                               message:@"namePrefix is empty"
                               details:nil]);
    return;
  }
  NSString *cacheDirectory = MarsRsXlogOptionalString(arguments, @"cacheDirectory");

  MarsXLogConfig config;
  config.mode = MarsRsXlogInt(arguments, @"mode", MarsAppenderAsync);
  config.log_dir = logDirectory.UTF8String;
  config.name_prefix = namePrefix.UTF8String;
  config.pub_key = MarsRsXlogString(arguments, @"publicKey").UTF8String;
  config.compress_mode = MarsRsXlogInt(arguments, @"compression", MarsCompressZlib);
  config.compress_level = MarsRsXlogInt(arguments, @"compressionLevel", 0);
  config.cache_dir = cacheDirectory.UTF8String;
  config.cache_days = MarsRsXlogInt(arguments, @"cacheDays", 0);

  int64_t handle =
      mars_xlog_new_instance(&config, MarsRsXlogInt(arguments, @"level", MarsLevelInfo));
  if (handle == 0) {
    result([FlutterError errorWithCode:@"mars_rs_xlog"
                               message:@"the appender refused the configuration"
                               details:nil]);
    return;
  }
  self.instance = handle;
  self.namePrefix = namePrefix;
  result(nil);
}

/// `mars_xlog_write_instance`. The file, the function and the line are left
/// empty: there is no Dart frame to name, and the C++ writes an empty one too.
- (void)write:(FlutterMethodCall *)call {
  NSDictionary *arguments = call.arguments;
  mars_xlog_write_instance(self.instance,
                           MarsRsXlogInt(arguments, @"level", MarsLevelInfo),
                           MarsRsXlogString(arguments, @"tag").UTF8String,
                           "",
                           "",
                           0,
                           MarsRsXlogString(arguments, @"message").UTF8String);
}

/// `mars_xlog_flush_instance`.
- (void)flush:(FlutterMethodCall *)call {
  NSDictionary *arguments = call.arguments;
  mars_xlog_flush_instance(self.instance, MarsRsXlogInt(arguments, @"sync", 0));
}

/// `mars_xlog_set_level_instance`.
- (void)setLevel:(FlutterMethodCall *)call {
  NSDictionary *arguments = call.arguments;
  mars_xlog_set_level_instance(self.instance, MarsRsXlogInt(arguments, @"level", MarsLevelInfo));
}

/// `mars_xlog_set_console_log_instance`.
- (void)setConsoleLog:(FlutterMethodCall *)call {
  NSDictionary *arguments = call.arguments;
  mars_xlog_set_console_log_instance(self.instance, MarsRsXlogInt(arguments, @"enabled", 0));
}

/// `mars_xlog_release_instance`: closes the appender `open` made.
- (void)close {
  mars_xlog_release_instance(self.namePrefix.UTF8String);
  self.instance = 0;
  self.namePrefix = @"";
}

@end
