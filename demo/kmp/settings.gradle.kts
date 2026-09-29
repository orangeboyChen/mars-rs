// The Gradle build of the Kotlin Multiplatform demo of mars-rs.
//
// Why a build of its own and not one more module of `platforms/kmp`: that one
// *is* the Kotlin Multiplatform package, and this is an app that takes it the
// way an app outside this repository does — from the registry, and with no
// Rust toolchain and no NDK anywhere in the build.
//
// Everything is the Kotlin DSL: `.gradle.kts` for the scripts and
// `gradle/libs.versions.toml` for the coordinates.

pluginManagement {
    repositories {
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        mavenCentral()
        // Where the Kotlin Multiplatform packages of the port are published:
        // `marsrs-kmp` for the whole port and `xlog-kmp` for the logging half.
        //
        // GitHub Packages answers nothing without a token — not even a 404 — so
        // the two below are read out of the environment and not out of a file: a
        // token written into `gradle.properties` is a token one `git add -A`
        // away from the remote, and this repository ignores that file for the
        // same reason. Any token with `read:packages` will do.
        //
        // The way in that needs no token at all is the release's
        // `marsrs-kmp-maven.zip`: unzip it anywhere and add
        // `maven { url = uri("<dir>") }` instead of this block.
        maven {
            url = uri("https://maven.pkg.github.com/orangeboyChen/mars-rs")
            credentials {
                username = System.getenv("GITHUB_ACTOR").orEmpty()
                password = System.getenv("GITHUB_TOKEN").orEmpty()
            }
        }
    }
}

rootProject.name = "marsrs-demo-kmp"
