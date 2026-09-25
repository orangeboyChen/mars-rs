// Top-level build file of the Android packaging of mars-rs.
//
// There is one module: `mars-xlog`, the AAR that carries `libmarsxlog.so` per
// ABI plus the Java class whose natives it implements. Nothing is compiled
// from Rust here — see the comment at the top of mars-xlog/build.gradle.kts.

plugins {
    // Declared here and applied in the module, so that the version of the
    // Android Gradle Plugin is the one thing there is to bump.
    id("com.android.library") version "9.4.1" apply false
}

tasks.register<Delete>("clean") {
    delete(rootProject.layout.buildDirectory)
}
