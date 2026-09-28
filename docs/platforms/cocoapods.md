# CocoaPods

```ruby
# Podfile
platform :ios, '12.0'
use_frameworks!

pod 'MarsRSXlog', :podspec => 'https://raw.githubusercontent.com/orangeboyChen/mars-rs/v0.1.0/MarsRSXlog.podspec'
```

Three pods, and they are the three products of [the Swift package](/platforms/swift)
under the same names:

| pod | what you get |
|---|---|
| `MarsRSXlog` | the logger: `Xlog`, `XlogConfig`, `LogLevel`, `AppenderMode`, `CompressMode` |
| `MarsRSNet` | [the task pipeline](/stn) and [the network diagnosis](/sdt): `MarsStn`, `MarsSdt`, `StnTask`, `StnQuestion`, `StnAnswer` |
| `MarsRS` | both halves, re-exported |

`:podspec` is the point of the line. The pods are not on a spec repo — a release
publishes `marsrs-cocoapods-xlog-<version>.zip` and
`marsrs-cocoapods-net-<version>.zip` beside the podspecs of the tag, and the
podspec is what names the archive `pod install` downloads. An app that writes
`pod 'MarsRSXlog'` alone is asking the trunk CDN for a pod that is not on it.

An app that wants both halves names all three:

```ruby
pod 'MarsRS', :podspec => 'https://raw.githubusercontent.com/orangeboyChen/mars-rs/v0.1.0/MarsRS.podspec'
pod 'MarsRSXlog', :podspec => 'https://raw.githubusercontent.com/orangeboyChen/mars-rs/v0.1.0/MarsRSXlog.podspec'
pod 'MarsRSNet', :podspec => 'https://raw.githubusercontent.com/orangeboyChen/mars-rs/v0.1.0/MarsRSNet.podspec'
```

`MarsRS` carries no framework of its own — what it is made of is two
`@_exported import` lines, one per half — so it reads the Swift of its module out
of the tag and the two frameworks out of the pods the other lines name.

`MarsRSXlog` and `MarsRSNet` are iOS 12.0 and watchOS 10.0, in Swift 5, and each
carries the same four slices the Swift package resolves. `MarsRSXlog` links
`CoreFoundation` on the app's behalf, which is what `iana-time-zone` asks the
time zone with and a Rust static library cannot name for itself.

## Swift

The Swift API is the one on [the SwiftPM page](/platforms/swift) — one `import`
and the same `Xlog`, `XlogConfig` and `LogLevel`, because the pod and the package
are the same Swift over the same framework.

## Objective-C

There is no Objective-C source in the port. `Xlog`, `XlogConfig` and the three
enums are `@objc`, and the header an Objective-C file imports is the one the
compiler writes out of them, into the pod's module:

```objc
@import MarsRSXlog;
```

```objc
XlogConfig *config = [[XlogConfig alloc] initWithLogDirectory:logDirectory.path];
config.namePrefix = @"marsrs";
config.level = LogLevelInfo;

NSError *error = nil;
Xlog *log = [[Xlog alloc] initWithConfig:config error:&error];
if (log == nil) {
    NSLog(@"no appender: %ld %@", (long)error.code, error.localizedDescription);
    return;
}
log.isConsoleLogEnabled = YES;

if ([log isEnabledFor:LogLevelDebug]) {
    [log writeWithLevel:LogLevelDebug message:@"cold start" tag:@"startup"];
}

[log flushWithSync:YES];   // before the app reads or uploads the files
[log close];
```

`initWithConfig:error:` is the throwing Swift initialiser: `nil` and an
`NSError` whose `code` is an `XlogError` — `XlogErrorEmptyLogDirectory`,
`XlogErrorEmptyNamePrefix`, `XlogErrorInvalidCompressionLevel`,
`XlogErrorNegativeCacheDays`, `XlogErrorRefused` — and whose
`localizedDescription` is the same message Swift's `description` answers with.
The domain is `MarsRSXlog.XlogError`.

`writeWithLevel:message:tag:` is what Objective-C writes a record with: Swift
fills `file`, `function` and `line` in at the call site, and Objective-C has no
`#file` to fill one with, so a record written here carries an empty file, an
empty function and the line 0. Where a record was written goes *in* the record
through the long form, which is where `__FILE__`, `__PRETTY_FUNCTION__` and
`__LINE__` go:

```objc
[log log:LogLevelInfo message:@"written from Objective-C" tag:@"objc"
        file:@__FILE__ function:@__PRETTY_FUNCTION__ line:__LINE__];
```

| what | how |
|---|---|
| move the level | `log.level = LogLevelWarning` |
| switch async / sync | `log.mode = AppenderModeSync` |
| mirror records to the console | `log.isConsoleLogEnabled = YES` |
| close a file at a size | `log.maxFileSizeBytes = 8 * 1024 * 1024` |
| drop a file at an age | `log.maxAliveTimeSeconds = 10 * 24 * 3600` |
| is it still open | `log.isOpen` |
| where the current file is | `[Xlog currentLogPath]` |

Every field of `XlogConfig` is settable the same way, and each of them is on
[the configuration page](/configuration):

```objc
config.mode = AppenderModeAsync;
config.compression = CompressModeZstd;
config.compressionLevel = 6;
config.cacheDays = 3;
config.publicKey = publicKey;   // empty writes an unencrypted file
```

What Objective-C does not get is the net half: `MarsStn` and `MarsSdt` are the
namespaces the C ABI's flat names are grouped under, and a Swift `enum` of static
members is not a type Objective-C can see. An app that runs tasks or a diagnosis
writes that part in Swift.
