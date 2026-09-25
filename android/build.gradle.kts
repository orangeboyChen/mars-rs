// Top-level build file of the Android packaging of mars-rs.
//
// Two modules over the same `libmarsxlog.so`: `mars-core`, the AAR that carries
// it per ABI plus every Java class whose natives it implements (xlog, STN, SDT,
// the app and the platform callbacks), and `mars-xlog`, the same library with
// the logging half of the Java only — the pair the C++ project publishes as
// `mars-core` and `mars-xlog`. Nothing is compiled from Rust here — see the
// comment at the top of mars-core/build.gradle.kts.

plugins {
    // Declared here and applied in the module, so that the version of the
    // Android Gradle Plugin is the one thing there is to bump.
    id("com.android.library") version "9.4.1" apply false
}

tasks.register<Delete>("clean") {
    delete(rootProject.layout.buildDirectory)
}
