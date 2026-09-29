// The Gradle build of the Android demo of mars-rs.
//
// Why a build of its own and not one more module of `platforms/android`: that
// one is a *library* build that publishes the two AARs, and an app is the other
// side of the line — it consumes them from JitPack the way an app outside this
// repository does, and nothing here compiles anything from Rust.
//
// Everything is the Kotlin DSL: `.gradle.kts` for the scripts and
// `gradle/libs.versions.toml` for the coordinates, which is the pair a project
// written today starts from.

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    // The repositories below are the only ones a module may resolve from, so a
    // module that wants one of its own is a build error and not a surprise.
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
        // Where the two AARs of the port are published: `marsrs` for the whole
        // port and `xlog` for the logging half. JitPack builds them from the
        // tag, which is why an app needs no NDK and no Rust toolchain of its
        // own — the `libmarsrsxlog.so` inside is already built.
        maven { url = uri("https://jitpack.io") }
    }
}

rootProject.name = "marsrs-demo-android"

include(":app")
