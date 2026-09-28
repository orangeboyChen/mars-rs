// The Android half of `marsrs-react-native`: a `ReactPackage` over `Xlog`, the
// Kotlin face of `libmarsrsxlog.so` in the `marsrs` AAR — the AAR of the whole
// port, which today is xlog. `marsrs-react-native-xlog` is the same module over
// `xlog`, the AAR an app that only logs takes, and the AAR is the whole
// difference between the two.
//
// Nothing is compiled from Rust here, and no `.so` is packaged either: the AAR
// of the release this module was packaged for carries them, which is what the
// dependency below is. `scripts/package_react_native.sh` stamps its version.

plugins {
    id("com.android.library")
    // No version, and no `apply false`: the app that takes this module is the
    // one that pins the Kotlin plugin, and Gradle resolves the request below
    // from there. The AAR modules of android/ need no plugin at all — their AGP
    // is pinned by this repository and is 9, which compiles Kotlin on its own —
    // but an app's is not, and is 8 today.
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "io.github.orangeboychen.marsrs.reactnative"
    compileSdk = 36

    defaultConfig {
        // React Native's own floor since 0.76, and above the 24 `mars-jni` is
        // built against.
        minSdk = 24
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
    // `react-android` and not `react-native`: the artifact React Native has
    // published since 0.71, and the one the app's own repository resolves.
    implementation("com.facebook.react:react-android")
    implementation("io.github.orangeboychen.marsrs:marsrs:0.1.0-alpha.2")
}
