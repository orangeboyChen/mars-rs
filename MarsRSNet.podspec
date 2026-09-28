# The `MarsRSNet` pod: the net half of the port — the diagnosis and the task
# pipeline — for an app that takes the port through CocoaPods.
#
# What the pod is made of is what the release packs into
# `marsrs-cocoapods-net-<version>.zip`: `marsrs-net.xcframework` — the same
# artefact the net binary target of Package.swift resolves — and the Swift of
# `platforms/apple/MarsRSNet`, which CocoaPods compiles into the pod's module.
# One archive, because a pod has one `source` and it is the only thing `pod
# install` fetches; see MarsRSXlog.podspec for why that is.
#
# The framework is the one built `--no-default-features --features sdt,stn`, so
# it carries `mars_sdt_*` and `mars_stn_*` and no `mars_xlog_*` at all: an app
# that only logs takes `MarsRSXlog` and downloads xlog alone, and an app that
# takes both pods links xlog's symbols exactly once.
#
# No `s.frameworks`, unlike the xlog pod: the net archive references nothing
# outside libSystem, which every app links. The xlog one names `CoreFoundation`
# because `iana-time-zone` asks `CFTimeZone*` what the time zone is, and a Rust
# static library carries no link flags of its own.
#
# No `@objc` on any of this half: `MarsStn` and `MarsSdt` are the namespaces the
# C ABI's flat names are grouped under, and a Swift `enum` of static members is
# not a type Objective-C can see. What an Objective-C app takes of the port is
# the logging half, which is the half that carries an Objective-C face.
#
# The version and the url are rewritten by .github/scripts/update_podspecs.py on
# every release, the way the binary targets of Package.swift are.

Pod::Spec.new do |s|
  s.name         = 'MarsRSNet'
  s.version      = '0.1.0-alpha.3'
  s.summary      = 'The net half of mars-rs: the Mars task pipeline and diagnosis, in Rust.'
  s.description  = <<-DESC
                   mars-rs is the Rust port of Mars, the cross-platform
                   infrastructure of WeChat. This pod carries the net half — the
                   STN task pipeline and the SDT network diagnosis — as a static
                   library for iOS and watchOS, with a Swift API over its C ABI:
                   `MarsStn` and `MarsSdt`. It carries no `mars_xlog_*` symbol,
                   so an app that takes both halves links xlog's exactly once.
                   DESC
  s.homepage     = 'https://github.com/orangeboyChen/mars-rs'
  s.license      = { :type => 'MIT', :file => 'LICENSE' }
  s.author       = { 'orangeboyChen' => 'https://github.com/orangeboyChen' }
  s.source       = { :http => 'https://github.com/orangeboyChen/mars-rs/releases/download/v0.1.0-alpha.3/marsrs-cocoapods-net-0.1.0-alpha.3.zip' }
  # What Package.swift declares, and what the four slices of the framework are
  # built at.
  s.platforms    = { :ios => '12.0', :watchos => '10.0' }
  # What Package.swift's `swift-tools-version` asks of the sources: they are
  # Swift 5, so that a Swift 5 project can depend on the port.
  s.swift_version = '5.0'
  s.source_files = 'platforms/apple/MarsRSNet/*.swift'
  s.vendored_frameworks = 'marsrs-net.xcframework'
end
