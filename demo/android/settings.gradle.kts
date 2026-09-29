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

// The appender this demo writes through, built out of the checkout beside it
// rather than taken out of JitPack.
//
// A composite build, and why one: `platforms/android` is the library build
// that publishes the two AARs, and this app is the other side of the line —
// it took them from JitPack the way an app outside this repository does,
// which meant the appender it compiled against was the one built from a tag
// and not the one in the tree beside it. A release one commit behind the
// sources is a demo that compiles here and not there, or the other way round:
// the AAR of the newest tag still answers `flush(sync = true)`, and this
// checkout answers `flushNow()`.
//
// The coordinate the app declares is unchanged, so what a reader copies is
// still the line an app outside this repository writes. The block is guarded
// for the same reason: a copy of `demo/android` on its own, with no
// `platforms/android` beside it, resolves from JitPack exactly as it did.
//
// What it needs before it will build is `libmarsrsxlog.so`, which
// platforms/android does not compile — see scripts/build_android.sh.
val localBuild = file("../../platforms/android")
if (localBuild.isDirectory) {
    includeBuild(localBuild) {
        dependencySubstitution {
            substitute(module("com.github.orangeboyChen.mars-rs:xlog"))
                .using(project(":marsrs-xlog"))
        }
    }
}

include(":app")
