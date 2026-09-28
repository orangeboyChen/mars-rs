// The Kotlin Multiplatform module of the whole port: `marsrs-kmp`, the pair
// `xlog-kmp` is the logging half of, the way the two crates on crates.io
// are a pair, `mars-core` and `mars-xlog` are the C++ project's, and `MarsRS`
// and `MarsRSXlog` are the pair `Package.swift` exposes.
//
// It declares nothing that is not xlog's, and that is the point: `marsrs-ffi`
// exports 28 `mars_xlog_*` symbols and nothing else today, so the whole port and
// its logging half are the same module — exactly the reason
// `marsrs-xlog.xcframework` is named after xlog and `MarsRS` in `Package.swift`
// has no symbols of its own. What this module is for is the coordinate: an app
// that depends on it gets xlog now and STN and SDT later, without a fourth
// artifact and without a rename.
//
// The API it exposes is `:marsrs-xlog`'s, re-exported twice over — by an `api`
// dependency, which is what puts `Xlog` on a consumer's classpath and the
// cinterop archive into a consumer's link, and by the typealiases of
// `src/commonMain`, which are what put the same names under *this* package and
// what Kotlin needs a source file for: a module with no source compiles to no
// klib, and a publication with no klib in it is one Gradle refuses to write.

plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.multiplatform")
    id("maven-publish")
}

val publishedGroup: String = (findProperty("publishedGroup") as String?) ?: "io.github.orangeboychen.marsrs"
val publishedArtifact: String = (findProperty("publishedArtifact") as String?) ?: "marsrs-kmp"
val publishedVersion: String = (findProperty("publishedVersion") as String?) ?: "0.0.0"

kotlin {
    jvmToolchain(17)

    androidTarget {
        publishLibraryVariants("release")
    }

    // The same targets as `marsrs-xlog`: a Kotlin Multiplatform publication is
    // one artifact per target, so a module that left one out would be a module
    // an app of that platform cannot resolve.
    iosArm64()
    iosX64()
    iosSimulatorArm64()
    macosX64()
    macosArm64()
    watchosArm64()
    watchosDeviceArm64()
    watchosSimulatorArm64()
    tvosArm64()
    tvosSimulatorArm64()
    linuxX64()
    linuxArm64()
    mingwX64()

    sourceSets {
        commonMain.dependencies {
            api(project(":marsrs-xlog"))
        }
    }
}

android {
    namespace = "io.github.orangeboychen.marsrs"
    compileSdk = 36

    defaultConfig {
        minSdk = 21
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildTypes {
        getByName("release") {
            isMinifyEnabled = false
        }
    }

    publishing {
        singleVariant("release") {
            withSourcesJar()
        }
    }
}

publishing {
    // GitHub Packages, the host `marsrs-xlog` is published to and the reason it
    // is named here too: Gradle publishes one module at a time, and this one is
    // the coordinate an app depends on.
    repositories {
        maven {
            name = "GitHubPackages"
            url = uri("https://maven.pkg.github.com/orangeboyChen/mars-rs")
            credentials {
                username = (findProperty("mavenUser") as String?) ?: System.getenv("GITHUB_ACTOR")
                password = (findProperty("mavenPassword") as String?) ?: System.getenv("GITHUB_TOKEN")
            }
        }
    }
}

// Named after the evaluation, for the same reason `marsrs-xlog` names its own
// there: the artifactId of the Android publication is AGP's to write, and it
// writes it after `publishing { }` has run.
afterEvaluate {
    publishing.publications.withType<MavenPublication>().configureEach {
        artifactId = if (name == "kotlinMultiplatform") {
            publishedArtifact
        } else {
            "$publishedArtifact-${name.lowercase()}"
        }
        groupId = publishedGroup
        version = publishedVersion

        pom {
            name.set(publishedArtifact)
            description.set("Kotlin Multiplatform library of the Rust port of Tencent/mars: xlog, STN and SDT")
            url.set("https://github.com/orangeboyChen/mars-rs")
            licenses {
                license {
                    name.set("MIT")
                    url.set("https://github.com/orangeboyChen/mars-rs/blob/main/LICENSE")
                }
            }
        }
    }
}
