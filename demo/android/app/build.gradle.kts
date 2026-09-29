// The app module of the Android demo: one screen that opens the appender,
// writes a record at every level, and says where the file went.
//
// The whole of what an app needs, and nothing of what the port does not:
// `libs.marsrs.xlog` is the AAR, and there is no NDK, no Rust toolchain and no
// `System.loadLibrary` in here — the AAR carries the `libmarsrsxlog.so` for
// `arm64-v8a`, `armeabi-v7a` and `x86_64`, and `Xlog` loads it.

plugins {
    alias(libs.plugins.android.application)
}

android {
    namespace = "io.github.orangeboychen.marsrs.demo"
    compileSdk = 36

    defaultConfig {
        applicationId = "io.github.orangeboychen.marsrs.demo"
        // The AAR's floor: `marsrs-jni` is built against NDK 27 / API 24, and
        // 21 is what the C++ project ships with.
        minSdk = 21
        targetSdk = 36
        versionCode = 1
        versionName = "1.0"
    }

    // `BuildConfig.DEBUG`, which is what the demo asks before it mirrors
    // records to logcat. AGP 9 leaves the generated class opted out unless a
    // build says it wants one.
    buildFeatures {
        buildConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // Kotlin's target, and the one thing a Kotlin module of AGP 9 has to set:
    // the compiler targets 1.8 unless it is told otherwise, and `marsrs-jni`
    // asks for 17. There is no Kotlin plugin to apply — see the comment in
    // `../build.gradle.kts`.
    kotlin {
        compilerOptions {
            jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
        }
    }

    buildTypes {
        getByName("release") {
            // Nothing to shrink in an app whose whole Kotlin is one Activity,
            // and the AAR ships the R8 rules it needs as `proguard.txt`, so an
            // app that does turn this on needs no rules of its own.
            isMinifyEnabled = false
        }
    }
}

dependencies {
    // The logging half of the port. The other line an app writes is
    // `libs.marsrs` — the whole port, xlog with STN and SDT beside it — and an
    // app takes one of the two.
    implementation(libs.marsrs.xlog)
}
