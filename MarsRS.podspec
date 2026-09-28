# The `MarsRS` pod: both halves of the port in one `import`.
#
# The umbrella, and the one pod of the three that carries no binary: what `MarsRS`
# is made of is `platforms/apple/MarsRS/MarsRS.swift` — two `@_exported import`
# lines, one per half — so its `source` is the git tag this version was
# published from and not an archive of a release. The two pods it names are the
# ones that carry the frameworks, and they are pinned to this version because a
# pod that
# resolved a newer half than the one it was published against is not the pair a
# release tested.
#
# Which is also why this is the one pod of the three with no framework of its own
# to vendor: an app that writes `import MarsRS` gets both halves, and an app that
# wants one takes `MarsRSXlog` or `MarsRSNet` and downloads that framework alone.
#
# The version is rewritten by .github/scripts/update_podspecs.py on every release,
# the way the binary targets of Package.swift are. Nothing else here moves: the
# tag is `v#{s.version}` — the tag of the version, which is the one the two pods
# below are published from as well.

Pod::Spec.new do |s|
  s.name         = 'MarsRS'
  s.version      = '0.1.0'
  s.summary      = 'mars-rs: the Rust port of Mars — xlog and the net half, in one module.'
  s.description  = <<-DESC
                   mars-rs is the Rust port of Mars, the cross-platform
                   infrastructure of WeChat. This pod is the umbrella over both
                   halves of the port: the xlog logger and the net half — the STN
                   task pipeline and the SDT network diagnosis — as static
                   libraries for iOS and watchOS, with a Swift API over their C
                   ABI. An app that wants one half takes `MarsRSXlog` or
                   `MarsRSNet` instead, and downloads that framework alone.
                   DESC
  s.homepage     = 'https://github.com/orangeboyChen/mars-rs'
  s.license      = { :type => 'MIT', :file => 'LICENSE' }
  s.author       = { 'orangeboyChen' => 'https://github.com/orangeboyChen' }
  s.source       = { :git => 'https://github.com/orangeboyChen/mars-rs.git',
                     :tag => "v#{s.version}" }
  # What Package.swift declares, and what the four slices of each framework are
  # built at.
  s.platforms    = { :ios => '12.0', :watchos => '10.0' }
  # What Package.swift's `swift-tools-version` asks of the sources: they are
  # Swift 5, so that a Swift 5 project can depend on the port.
  s.swift_version = '5.0'
  s.source_files = 'platforms/apple/MarsRS/*.swift'
  # The two halves, at the version of this pod: an umbrella that resolved a newer
  # half than it was published against is not the pair a release tested.
  s.dependency 'MarsRSXlog', s.version.to_s
  s.dependency 'MarsRSNet', s.version.to_s
end
