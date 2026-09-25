// The xlog-only Android AAR of mars-rs: `libmarsxlog.so` (crate `mars-jni`)
// for every ABI, plus the two Kotlin classes of the logging half — `Xlog` and
// the `Log` facade over it. The whole port is the `mars-core` module; this is the
// package for an app that only logs, the way the C++ project's `mars-xlog` is.
//
// Its two source files are byte-for-byte mars-core's — the same xlog API over
// the same `.so`, with the rest of the port left out — so a change to one
// belongs in both. They are copied and not shared because a module that took mars-core's
// sources would take all of them, which is what this module exists not to do.
//
// The .so files are not built here. Neither JitPack nor a plain `./gradlew`
// has an NDK and a Rust toolchain, so .github/workflows/release.yml builds
// them and publishes `mars-android-native.zip` with the release; that zip (see
// ../../jitpack.yml, and the `android` job of the workflow) is what puts them
// in `libs/<abi>/`.

plugins {
    id("com.android.library")
    // No `org.jetbrains.kotlin.android`: AGP 9 compiles Kotlin on its own and
    // refuses the plugin (issuetracker.google.com/438678642).
    id("maven-publish")
}

// `mars-jni` asks for Java 17 bytecode, and the Kotlin compiler targets 1.8
// unless it is told otherwise.
kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

val publishedGroup: String = (findProperty("publishedGroup") as String?) ?: "io.github.orangeboychen"
val publishedArtifact: String = (findProperty("publishedArtifact") as String?) ?: "mars-rs-xlog"
val publishedVersion: String = (findProperty("publishedVersion") as String?) ?: "0.0.0"

android {
    namespace = "io.github.orangeboychen.marsrs.xlog"
    compileSdk = 36

    defaultConfig {
        // `mars-jni` is built against NDK 27 / API 24; 21 is the floor the C++
        // project ships with.
        minSdk = 21
    }

    sourceSets {
        getByName("main") {
            jniLibs.srcDirs("libs")
        }
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

    // AGP publishes nothing until a variant is named: this is what makes
    // `components["release"]` exist for the publication below.
    publishing {
        singleVariant("release") {
            withSourcesJar()
        }
    }
}

// `components["release"]` does not exist yet while this script runs: AGP
// registers it in an `afterEvaluate` of its own, so the publication is
// declared now and pointed at the component later.
publishing {
    publications {
        register<MavenPublication>("aar") {
            groupId = publishedGroup
            artifactId = publishedArtifact
            version = publishedVersion

            pom {
                packaging = "aar"
                name.set("mars-rs-xlog")
                description.set("Android AAR of the Rust port of Tencent/mars: xlog")
                url.set("https://github.com/orangeboyChen/mars-rs")
                licenses {
                    license {
                        name.set("MIT")
                        url.set("https://github.com/orangeboyChen/mars-rs/blob/main/LICENSE")
                    }
                }
            }

            afterEvaluate {
                from(components["release"])
            }
        }
    }
}

afterEvaluate {
    // An AAR without a .so in it is worthless, and JitPack would publish it all
    // the same, so fail loudly instead. The check is orangeboyChen/mars'.
    val abis = file("libs").listFiles()?.filter { it.isDirectory } ?: emptyList()
    if (abis.isEmpty()) {
        throw GradleException(
            "No native libraries found in ${file("libs")}. Unpack mars-android-native.zip " +
                "of the release into it first (see jitpack.yml and the 'android' job of " +
                ".github/workflows/release.yml)."
        )
    }
    println("mars-xlog: packaging ${abis.map { it.name }.sorted().joinToString(", ")}")
}
