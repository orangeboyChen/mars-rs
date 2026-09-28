// The Android half of the `marsrs_flutter` plugin: a method channel over `Xlog`, the
// Kotlin face of `libmarsrsxlog.so` in the `marsrs` AAR — the AAR of the whole
// port, which today is xlog. `marsrs_flutter_xlog` is the same plugin over
// `xlog`, the AAR an app that only logs takes, and the AAR is the whole
// difference between the two.
//
// Nothing is compiled from Rust here, and no `.so` is packaged either: the AAR
// of the release this plugin was packaged for carries them, which is what the
// dependency below is. `scripts/package_flutter.sh` stamps its version.

plugins {
    id("com.android.library")
    // No version, and no `apply false`: the app that takes this plugin is the
    // one that pins the Kotlin plugin, in the `plugins {}` block of its
    // settings.gradle.kts, and Gradle resolves the request below from there.
    // The AAR modules of android/ need no plugin at all — their AGP is pinned
    // by this repository and is 9, which compiles Kotlin on its own — but a
    // Flutter app's is not, and is 8 today.
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "io.github.orangeboychen.marsrs.flutter"
    compileSdk = 36

    defaultConfig {
        // `mars-jni` is built against NDK 27 / API 24; 21 is the floor the C++
        // project ships with.
        minSdk = 21
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

repositories {
    google()
    mavenCentral()
    // `marsrs`, the AAR of the release; JitPack is where it is published.
    maven("https://jitpack.io")
}

dependencies {
    implementation("io.github.orangeboychen.marsrs:marsrs:0.1.0-alpha.2")
}
