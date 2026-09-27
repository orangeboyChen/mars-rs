// The Kotlin Multiplatform build of mars-rs: one Gradle build of its own, next
// to the Android one in `../android`.
//
// Why a build of its own and not two more modules of `../android`: the AARs
// there are compiled by Android Gradle Plugin 9, which brings the Kotlin
// compiler with it and refuses a Kotlin plugin of its own
// (issuetracker.google.com/438678642). A Kotlin Multiplatform module needs
// exactly that plugin — `org.jetbrains.kotlin.multiplatform` — at a version
// this project chooses, so the two cannot share one build.
//
// Nothing is compiled from Rust here either: see the comment at the top of
// mars-xlog/build.gradle.kts.

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "mars-rs-kmp"

// The pair the C++ project publishes as `mars-core` and `mars-xlog`, and the
// pair `Package.swift` exposes as `MarsRS` and `MarsRSXlog`: `mars-core` is the
// whole port — today xlog, because that is all `mars-ffi` exports — and
// `mars-xlog` is the logging half, the module that does the work.
include(":mars-core")
include(":mars-xlog")
