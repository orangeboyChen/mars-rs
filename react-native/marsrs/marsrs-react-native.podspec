# The iOS half of `marsrs-react-native`.
#
# `MarsRSXlog.xcframework` is carried here and not resolved: CocoaPods cannot
# take the SwiftPM binary target of Package.swift, and there is no pod for it,
# so `.github/workflows/release.yml` drops the release's framework into
# `ios/Frameworks/` before the package is packed. `ios/include/mars_xlog.h` is
# the C ABI's own header, copied by `scripts/package_react_native.sh` out of
# `crates/marsrs-ffi/include` rather than kept here, so it cannot drift from it.
#
# The glue is Objective-C because that is what the header is written in: the
# framework is a static library with a C header, and `MarsRSFFI` — the module
# SwiftPM makes of the two — is nothing CocoaPods can hand a pod.
#
# The version is package.json's, and not a number of its own: one place a
# release stamps.

require "json"

package = JSON.parse(File.read(File.join(__dir__, "package.json")))

Pod::Spec.new do |s|
  s.name         = package["name"]
  s.version      = package["version"]
  s.summary      = package["description"]
  s.homepage     = package["homepage"]
  s.license      = package["license"]
  s.author       = { "orangeboyChen" => "https://github.com/orangeboyChen" }
  s.source       = { :git => "https://github.com/orangeboyChen/mars-rs.git",
                     :tag => "v#{s.version}" }
  # What Package.swift declares for iOS; the framework's own floor.
  s.platform     = :ios, "12.0"
  s.source_files = "ios/*.{h,m}"
  s.vendored_frameworks = "ios/Frameworks/MarsRSXlog.xcframework"
  s.preserve_paths = "ios/include/mars_xlog.h"
  s.xcconfig     = { "HEADER_SEARCH_PATHS" => "\"$(PODS_TARGET_SRCROOT)/ios/include\"" }
  # `iana-time-zone` calls `CFTimeZone*`, and a Rust static library carries no
  # link flags of its own: Package.swift names the framework for the same reason.
  s.frameworks   = "CoreFoundation"
  # React Native's own, in place of `s.dependency "React-Core"`: it is what the
  # app's `use_react_native!` provides, and it keeps the pod off a version of
  # React Native the app did not choose.
  install_modules_dependencies(s)
end
