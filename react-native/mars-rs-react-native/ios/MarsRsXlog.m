#import "MarsRsXlog.h"

#import "mars_xlog.h"

// The iOS half of `mars-rs-react-native`: the C ABI of `mars_xlog.h` (crate
// `mars-ffi`) behind the six methods of the `MarsRsXlog` native module.
//
// Objective-C, and not Swift: what the module carries is a static library with
// a C header, and `MarsRSFFI` — the module SwiftPM makes of the two — is not
// something CocoaPods can hand a pod, so the glue is written against the header
// itself. It is the same instance API `MarsXlogInstance` of
// `Sources/MarsRSXlog/Xlog.swift` is: `mars_xlog_new_instance` and friends.
//
// Every field below is one `src/index.ts` sent, and every default is the one
// `mars_xlog.h` documents for the field — `log_dir` is the one with no default,
// because the appender answers `MARS_XLOG_ERR_EMPTY_LOG_DIR` without it.

static NSString *const kMarsRsXlogError = @"mars_rs_xlog";

/// The number sent for `key`, or `fallback` when none was sent. A field
/// `src/index.ts` left out is not one it sent as `null`.
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

@interface MarsRsXlog ()

/// The instance `mars_xlog_new_instance` gave; `0` before `open`, after `close`.
@property(nonatomic) int64_t instance;

/// What `close` releases the instance by: the prefix it was opened with.
@property(nonatomic, copy) NSString *namePrefix;

@end

@implementation MarsRsXlog

// `MarsRsXlog`, and no argument: the name `src/index.ts` reads the module out of
// `NativeModules` by is the class's.
RCT_EXPORT_MODULE()

/// `mars_xlog_new_instance`. `0` is the answer for a configuration the appender
/// refused, and an instance the caller has no handle to is one every write
/// would go to the process-wide appender instead of.
RCT_EXPORT_METHOD(open:(NSDictionary *)config
                 resolver:(RCTPromiseResolveBlock)resolve
                 rejecter:(RCTPromiseRejectBlock)reject) {
  NSString *logDirectory = MarsRsXlogString(config, @"logDirectory");
  if (logDirectory.length == 0) {
    reject(kMarsRsXlogError, @"logDirectory is empty", nil);
    return;
  }
  NSString *namePrefix = MarsRsXlogString(config, @"namePrefix");
  NSString *cacheDirectory = MarsRsXlogOptionalString(config, @"cacheDirectory");

  MarsXLogConfig xlogConfig;
  xlogConfig.mode = MarsRsXlogInt(config, @"mode", MarsAppenderAsync);
  xlogConfig.log_dir = logDirectory.UTF8String;
  xlogConfig.name_prefix = namePrefix.UTF8String;
  xlogConfig.pub_key = MarsRsXlogString(config, @"publicKey").UTF8String;
  xlogConfig.compress_mode = MarsRsXlogInt(config, @"compression", MarsCompressZlib);
  xlogConfig.compress_level = MarsRsXlogInt(config, @"compressionLevel", 0);
  xlogConfig.cache_dir = cacheDirectory.UTF8String;
  xlogConfig.cache_days = MarsRsXlogInt(config, @"cacheDays", 0);

  int64_t handle =
      mars_xlog_new_instance(&xlogConfig, MarsRsXlogInt(config, @"level", MarsLevelInfo));
  if (handle == 0) {
    reject(kMarsRsXlogError, @"the appender refused the configuration", nil);
    return;
  }
  self.instance = handle;
  self.namePrefix = namePrefix;
  resolve(nil);
}

/// `mars_xlog_write_instance`. The file, the function and the line are left
/// empty: there is no JS frame to name, and the C++ writes an empty one too.
RCT_EXPORT_METHOD(write:(int)level
                    tag:(NSString *)tag
                message:(NSString *)message
               resolver:(RCTPromiseResolveBlock)resolve
               rejecter:(RCTPromiseRejectBlock)reject) {
  mars_xlog_write_instance(self.instance, level, tag.UTF8String, "", "", 0, message.UTF8String);
  resolve(nil);
}

/// `mars_xlog_flush_instance`.
RCT_EXPORT_METHOD(flush:(BOOL)sync
                 resolver:(RCTPromiseResolveBlock)resolve
                 rejecter:(RCTPromiseRejectBlock)reject) {
  mars_xlog_flush_instance(self.instance, sync ? 1 : 0);
  resolve(nil);
}

/// `mars_xlog_set_level_instance`.
RCT_EXPORT_METHOD(setLevel:(int)level
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject) {
  mars_xlog_set_level_instance(self.instance, level);
  resolve(nil);
}

/// `mars_xlog_set_console_log_instance`.
RCT_EXPORT_METHOD(setConsoleLog:(BOOL)enabled
                       resolver:(RCTPromiseResolveBlock)resolve
                       rejecter:(RCTPromiseRejectBlock)reject) {
  mars_xlog_set_console_log_instance(self.instance, enabled ? 1 : 0);
  resolve(nil);
}

/// `mars_xlog_release_instance`: closes the appender `open` made.
RCT_EXPORT_METHOD(close:(RCTPromiseResolveBlock)resolve
                 rejecter:(RCTPromiseRejectBlock)reject) {
  mars_xlog_release_instance(self.namePrefix.UTF8String);
  self.instance = 0;
  self.namePrefix = @"";
  resolve(nil);
}

@end
