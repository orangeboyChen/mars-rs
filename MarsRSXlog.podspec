# The `MarsRSXlog` pod: the logging half of the port, for an app that takes the
# port through CocoaPods.
#
# What the pod is made of is what the release packs into
# `marsrs-cocoapods-xlog-<version>.zip`: `marsrs-xlog.xcframework` — the same
# artefact the xlog binary target of Package.swift resolves, down to the checksum
# a release computes for it — and the Swift of `platforms/apple/MarsRSXlog`,
# which CocoaPods compiles into the pod's module. One archive and not two
# downloads, because a pod has one `source` and it is the only thing `pod
# install` fetches: SwiftPM reads a manifest from a tag and a framework from a
# url, and CocoaPods reads a pod's files from its source and from nowhere
# else.
#
# The Swift is compiled by the app's Xcode and not by the release, which is what
# makes this pod the Objective-C seam of the port too: `Xlog` and `XlogConfig`
# are `@objc`, and the header an Objective-C file imports is the one the compiler
# writes out of them — `@import MarsRSXlog;`, or
# `#import <MarsRSXlog/MarsRSXlog-Swift.h>`. There is no Objective-C source in
# the port; the linkage is Swift's.
#
# `vendored_frameworks` is all the framework asks of a podspec: every slice of
# the xcframework carries the `Headers` directory `scripts/build_xcframework.sh`
# wrote its module map into, and Xcode puts the slice it resolves on the include
# path — so `import MarsRSFFI`, in the Swift of this pod, resolves without a
# search path of ours. Naming a module map of our own is what breaks it: two
# module maps declaring one module is a redefinition, and the framework's is
# already there.
#
# `CoreFoundation` is the framework the Rust static library needs and cannot name
# for itself: `iana-time-zone` asks `CFTimeZone*` what the time zone is, and a
# static library carries no link flags. Package.swift names it for the same
# reason.
#
# The version and the url are rewritten by .github/scripts/update_podspecs.py on
# every release, the way the binary targets of Package.swift are: a pod names the
# version it is and the file `pod install` downloads, and neither is a number
# this file can carry of its own.

Pod::Spec.new do |s|
  s.name         = 'MarsRSXlog'
  s.version      = '0.1.0'
  s.summary      = 'xlog, the logging half of mars-rs: the Mars logger in Rust.'
  s.description  = <<-DESC
                   mars-rs is the Rust port of Mars, the cross-platform
                   infrastructure of WeChat. This pod carries xlog — the logger —
                   as a static library for iOS and watchOS, with a Swift API over
                   its C ABI and an Objective-C one over the Swift: `Xlog`,
                   `XlogConfig` and `LogLevel`. An app that only logs takes this
                   one; `MarsRSNet` is the other half and `MarsRS` is both.
                   DESC
  s.homepage     = 'https://github.com/orangeboyChen/mars-rs'
  s.license      = { :type => 'MIT', :file => 'LICENSE' }
  s.author       = { 'orangeboyChen' => 'https://github.com/orangeboyChen' }
  s.source       = { :http => 'https://github.com/orangeboyChen/mars-rs/releases/download/v0.1.0/marsrs-cocoapods-xlog-0.1.0.zip' }
  # What Package.swift declares, and what the four slices of the framework are
  # built at.
  s.platforms    = { :ios => '12.0', :watchos => '10.0' }
  # What Package.swift's `swift-tools-version` asks of the sources: they are
  # Swift 5, so that a Swift 5 project can depend on the port.
  s.swift_version = '5.0'
  s.source_files = 'platforms/apple/MarsRSXlog/*.swift'
  s.vendored_frameworks = 'marsrs-xlog.xcframework'
  s.frameworks   = 'CoreFoundation'
end
