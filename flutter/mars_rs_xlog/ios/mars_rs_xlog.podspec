# The iOS half of the `mars_rs_xlog` plugin.
#
# `MarsRSXlog.xcframework` is carried here and not resolved: CocoaPods cannot
# take the SwiftPM binary target of Package.swift, and there is no pod for it,
# so `.github/workflows/release.yml` drops the release's framework into
# `Frameworks/` before the plugin is packaged. `include/mars_xlog.h` is the C
# ABI's own header, copied by `scripts/package_flutter.sh` out of
# `crates/mars-ffi/include` rather than kept here, so it cannot drift from it.
#
# The glue is Objective-C because that is what the header is written in: the
# framework is a static library with a C header, and `MarsRSFFI` — the module
# SwiftPM makes of the two — is nothing CocoaPods can hand a pod.

Pod::Spec.new do |s|
  s.name             = 'mars_rs_xlog'
  s.version          = '0.1.0-alpha.2'
  s.summary          = 'Flutter plugin for xlog, the logging half of mars-rs.'
  s.homepage         = 'https://github.com/orangeboyChen/mars-rs'
  s.license          = { :type => 'MIT', :file => '../LICENSE' }
  s.author           = { 'orangeboyChen' => 'https://github.com/orangeboyChen' }
  s.source           = { :path => '.' }
  # What Package.swift declares for iOS; the framework's own floor.
  s.platform         = :ios, '12.0'
  s.source_files     = 'Classes/**/*.{h,m}'
  s.vendored_frameworks = 'Frameworks/MarsRSXlog.xcframework'
  s.preserve_paths   = 'include/mars_xlog.h'
  s.xcconfig         = { 'HEADER_SEARCH_PATHS' => '"$(PODS_TARGET_SRCROOT)/include"' }
  # `iana-time-zone` calls `CFTimeZone*`, and a Rust static library carries no
  # link flags of its own: Package.swift names the framework for the same reason.
  s.frameworks       = 'CoreFoundation'
  s.dependency 'Flutter'
end
