// The Android AAR of the whole port: `libmarsrsxlog.so` (crate `marsrs-jni`) for
// every ABI, plus every Kotlin class whose natives it implements — xlog, STN,
// SDT, the app and the platform callbacks. `marsrs-xlog` is the same library with
// the xlog half of the Kotlin only: the pair the two crates on crates.io are,
// and the pair the C++ project publishes as `mars-core` and `mars-xlog`.
//
// The .so files are not built here. Neither JitPack nor a plain `./gradlew`
// has an NDK and a Rust toolchain, so .github/workflows/release.yml builds
// them and publishes `marsrs-android-native.zip` with the release; that zip (see
// jitpack.yml, and the `android` job of the workflow) is what puts them
// in `libs/<abi>/`.

plugins {
    id("com.android.library")
    // No `org.jetbrains.kotlin.android`: AGP 9 compiles Kotlin on its own and
    // refuses the plugin (issuetracker.google.com/438678642).
    id("maven-publish")
}

// `marsrs-jni` asks for Java 17 bytecode, and the Kotlin compiler targets 1.8
// unless it is told otherwise.
kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

val publishedGroup: String = (findProperty("publishedGroup") as String?) ?: "io.github.orangeboychen.marsrs"
val publishedArtifact: String = (findProperty("publishedArtifact") as String?) ?: "marsrs"
val publishedVersion: String = (findProperty("publishedVersion") as String?) ?: "0.0.0"

android {
    namespace = "io.github.orangeboychen.marsrs"
    compileSdk = 36

    defaultConfig {
        // `marsrs-jni` is built against NDK 27 / API 24; 21 is the floor the C++
        // project ships with.
        minSdk = 21

        // The rules an app's R8 needs to keep the JNI interface whole, carried
        // in the AAR as `proguard.txt` and merged into the app's own rules by
        // AGP: the .so and this Kotlin name each other, and a release build
        // that shrinks renames both halves. Without them an app that sets
        // `minifyEnabled true` — which is what a release build is — gets
        // `UnsatisfiedLinkError` and `NoSuchFieldError` at the first call.
        consumerProguardFiles("consumer-rules.pro")
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

// The one dependency this module has, and only because `flush()` hands the
// drain to another thread: a `suspend` function and `Continuation` are in the
// standard library, but what parks the caller until a thread of the I/O pool is
// done is not. `implementation` and not `api`: nothing of kotlinx-coroutines is
// in the API this module publishes — a `suspend` function is a method with a
// `Continuation` parameter to its caller, and that is the standard library's.
dependencies {
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2")
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
                name.set("marsrs")
                description.set("Android AAR of the Rust port of Tencent/mars: xlog, STN and SDT")
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
            "No native libraries found in ${file("libs")}. Unpack marsrs-android-native.zip " +
                "of the release into it first (see jitpack.yml and the 'android' job of " +
                ".github/workflows/release.yml)."
        )
    }
    println("marsrs: packaging ${abis.map { it.name }.sorted().joinToString(", ")}")
}
