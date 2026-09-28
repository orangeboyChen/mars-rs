// What the Swift of the pod sees of the C ABI: `mars_xlog.h`, copied out of
// `crates/marsrs-ffi/include` by `scripts/package_react_native.sh` rather than
// kept here, so it cannot drift from the header the Rust is built against.
//
// A bridging header and not an `import`, because the framework the pod carries
// is a static library with a C header: `MarsRSFFI`, the module SwiftPM makes of
// the two, is nothing CocoaPods can hand a pod.
//
// React Native is not here, and that is the point: `Xlog.swift` conforms to no
// React Native protocol, because the one it would conform to — `NativeXlogSpec`,
// which codegen wrote — is declared in a header that refuses to be imported by
// anything but Objective-C++. What the module owes React Native instead is in
// `Xlog.mm`, which says why there.
#include "mars_xlog.h"
