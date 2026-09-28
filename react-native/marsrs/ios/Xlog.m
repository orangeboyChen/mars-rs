#import "Xlog.h"

#import "mars_xlog.h"

// The iOS half of `marsrs-react-native`: the C ABI of `mars_xlog.h` (crate
// `marsrs-ffi`) behind the eleven methods of the `Xlog` native module.
//
// Objective-C, and not Swift: what the module carries is a static library with
// a C header, and `MarsRSFFI` — the module SwiftPM makes of the two — is not
// something CocoaPods can hand a pod, so the glue is written against the header
// itself. It is the instance API every other platform of the port is written
// against: `mars_xlog_new_instance` and friends, one appender per prefix.
//
// The xlog-only module is `marsrs-react-native-xlog`, and this file is its
// Objective-C with the two names changed: xlog is the whole C ABI today, so
// the two are one package under two names.
//
// Every key below is a field `src/index.ts` put there, and every default is the
// one `mars_xlog.h` documents — `logDir` is the field with none, because
// the appender answers `MARS_XLOG_ERR_EMPTY_LOG_DIR` without it.

/// What every `reject` below answers as its code: the package a JS caller sees
/// the rejection come from.
static NSString *const kXlogError = @"marsrs-react-native";

/// `XlogConfig.namePrefix` of the Kotlin, of the Swift and of the TS: what an
/// appender is opened with when the caller gave none.
static NSString *const kXlogDefaultNamePrefix = @"xlog";

/// The number sent for `key`, or `fallback` when none was sent. A field
/// `src/index.ts` left out is not one it sent as `null`.
static int XlogInt(NSDictionary *arguments, NSString *key, int fallback) {
  NSNumber *value = arguments[key];
  return [value isKindOfClass:NSNumber.class] ? value.intValue : fallback;
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

@interface Xlog ()

/// The appender of every prefix `open` has opened, by the prefix: what keeps
/// two `Xlog`s apart, which one handle cannot, because the appender a
/// `mars_xlog_release_instance` releases is the one of the prefix it is given.
@property(nonatomic, strong) NSMutableDictionary<NSString *, NSNumber *> *instances;

@end

@implementation Xlog

// `Xlog`, and no argument: the name `src/index.ts` reads the module out of
// `NativeModules` by is the class's.
RCT_EXPORT_MODULE()

- (instancetype)init {
  self = [super init];
  if (self) {
    _instances = [[NSMutableDictionary alloc] init];
  }
  return self;
}

/// `mars_xlog_new_instance`. `0` is the answer for a configuration the appender
/// refused, and an instance the caller has no handle to is one every write
/// would go to the process-wide appender instead of.
RCT_EXPORT_METHOD(open:(NSDictionary *)config
                 resolver:(RCTPromiseResolveBlock)resolve
                 rejecter:(RCTPromiseRejectBlock)reject) {
  NSString *logDir = XlogString(config, @"logDir");
  if (logDir.length == 0) {
    reject(kXlogError, @"logDir is empty", nil);
    return;
  }
  NSString *namePrefix = XlogString(config, @"namePrefix");
  if (namePrefix.length == 0) {
    namePrefix = kXlogDefaultNamePrefix;
  }

  MarsXLogConfig xlogConfig;
  xlogConfig.mode = XlogInt(config, @"mode", MarsAppenderAsync);
  xlogConfig.log_dir = logDir.UTF8String;
  xlogConfig.name_prefix = namePrefix.UTF8String;
  xlogConfig.pub_key = XlogString(config, @"pubKey").UTF8String;
  xlogConfig.compress_mode = XlogInt(config, @"compressMode", MarsCompressZlib);
  xlogConfig.compress_level = XlogInt(config, @"compressLevel", 0);
  xlogConfig.cache_dir = XlogOptionalString(config, @"cacheDir").UTF8String;
  xlogConfig.cache_days = XlogInt(config, @"cacheDays", 0);

  long long handle = mars_xlog_new_instance(&xlogConfig, XlogInt(config, @"level", MarsLevelInfo));
  if (handle == 0) {
    reject(kXlogError, @"the appender refused the configuration", nil);
    return;
  }
  self.instances[namePrefix] = @(handle);
  resolve(nil);
}

/// `mars_xlog_write_instance`. The file, the function and the line are left
/// empty: there is no JS frame to name, and the C++ writes an empty one too.
RCT_EXPORT_METHOD(log:(NSString *)namePrefix
                   level:(int)level
                     tag:(NSString *)tag
                 message:(NSString *)message
                resolver:(RCTPromiseResolveBlock)resolve
                rejecter:(RCTPromiseRejectBlock)reject) {
  NSNumber *handle = [self instanceForPrefix:namePrefix rejecter:reject];
  if (handle == nil) {
    return;
  }
  mars_xlog_write_instance(handle.longLongValue, level, tag.UTF8String, "", "", 0, message.UTF8String);
  resolve(nil);
}

/// `mars_xlog_is_enabled_for`: whether a record of the level would be written,
/// which an app asks before it builds a message that is expensive to build.
RCT_EXPORT_METHOD(isLoggable:(NSString *)namePrefix
                       level:(int)level
                    resolver:(RCTPromiseResolveBlock)resolve
                    rejecter:(RCTPromiseRejectBlock)reject) {
  NSNumber *handle = [self instanceForPrefix:namePrefix rejecter:reject];
  if (handle == nil) {
    return;
  }
  resolve(@(mars_xlog_is_enabled_for(handle.longLongValue, level) != 0));
}

/// `mars_xlog_flush_instance`.
RCT_EXPORT_METHOD(flush:(NSString *)namePrefix
                    sync:(BOOL)sync
                resolver:(RCTPromiseResolveBlock)resolve
                rejecter:(RCTPromiseRejectBlock)reject) {
  NSNumber *handle = [self instanceForPrefix:namePrefix rejecter:reject];
  if (handle == nil) {
    return;
  }
  mars_xlog_flush_instance(handle.longLongValue, sync ? 1 : 0);
  resolve(nil);
}

/// `mars_xlog_set_level_instance`.
RCT_EXPORT_METHOD(setLevel:(NSString *)namePrefix
                     level:(int)level
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject) {
  NSNumber *handle = [self instanceForPrefix:namePrefix rejecter:reject];
  if (handle == nil) {
    return;
  }
  mars_xlog_set_level_instance(handle.longLongValue, level);
  resolve(nil);
}

/// `mars_xlog_get_level`: what the appender answers, and not what JS holds.
RCT_EXPORT_METHOD(getLevel:(NSString *)namePrefix
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject) {
  NSNumber *handle = [self instanceForPrefix:namePrefix rejecter:reject];
  if (handle == nil) {
    return;
  }
  resolve(@(mars_xlog_get_level(handle.longLongValue)));
}

/// `mars_xlog_set_mode_instance`.
RCT_EXPORT_METHOD(setMode:(NSString *)namePrefix
                     mode:(int)mode
                 resolver:(RCTPromiseResolveBlock)resolve
                 rejecter:(RCTPromiseRejectBlock)reject) {
  NSNumber *handle = [self instanceForPrefix:namePrefix rejecter:reject];
  if (handle == nil) {
    return;
  }
  mars_xlog_set_mode_instance(handle.longLongValue, mode);
  resolve(nil);
}

/// `mars_xlog_set_console_log_instance`.
RCT_EXPORT_METHOD(setConsoleLogEnabled:(NSString *)namePrefix
                        enabled:(BOOL)enabled
                       resolver:(RCTPromiseResolveBlock)resolve
                       rejecter:(RCTPromiseRejectBlock)reject) {
  NSNumber *handle = [self instanceForPrefix:namePrefix rejecter:reject];
  if (handle == nil) {
    return;
  }
  mars_xlog_set_console_log_instance(handle.longLongValue, enabled ? 1 : 0);
  resolve(nil);
}

/// `mars_xlog_set_max_file_size_instance`. A `double` and not a `uint64_t`: the
/// bridge carries every JS number as one, and a file size is below 2^53.
RCT_EXPORT_METHOD(setMaxFileSize:(NSString *)namePrefix
                           bytes:(double)bytes
                        resolver:(RCTPromiseResolveBlock)resolve
                        rejecter:(RCTPromiseRejectBlock)reject) {
  NSNumber *handle = [self instanceForPrefix:namePrefix rejecter:reject];
  if (handle == nil) {
    return;
  }
  mars_xlog_set_max_file_size_instance(handle.longLongValue, (unsigned long long)bytes);
  resolve(nil);
}

/// `mars_xlog_set_max_alive_duration_instance`.
RCT_EXPORT_METHOD(setMaxAliveTime:(NSString *)namePrefix
                          seconds:(double)seconds
                         resolver:(RCTPromiseResolveBlock)resolve
                         rejecter:(RCTPromiseRejectBlock)reject) {
  NSNumber *handle = [self instanceForPrefix:namePrefix rejecter:reject];
  if (handle == nil) {
    return;
  }
  mars_xlog_set_max_alive_duration_instance(handle.longLongValue, (long long)seconds);
  resolve(nil);
}

/// `mars_xlog_release_instance`: closes the appender `open` made.
RCT_EXPORT_METHOD(close:(NSString *)namePrefix
                 resolver:(RCTPromiseResolveBlock)resolve
                 rejecter:(RCTPromiseRejectBlock)reject) {
  if (self.instances[namePrefix] == nil) {
    resolve(nil);
    return;
  }
  mars_xlog_release_instance(namePrefix.UTF8String);
  [self.instances removeObjectForKey:namePrefix];
  resolve(nil);
}

#pragma mark - The appender of a call

/// The handle of the appender of `namePrefix`, or `nil` with `reject` called:
/// no handle is the process-wide appender to the C ABI, so a call that went on
/// with one would write through whatever appender the rest of the process
/// writes through.
- (NSNumber *)instanceForPrefix:(NSString *)namePrefix rejecter:(RCTPromiseRejectBlock)reject {
  NSNumber *handle = self.instances[namePrefix];
  if (handle == nil) {
    reject(kXlogError,
           [NSString stringWithFormat:@"no appender of '%@' is open", namePrefix],
           nil);
    return nil;
  }
  return handle;
}

@end
