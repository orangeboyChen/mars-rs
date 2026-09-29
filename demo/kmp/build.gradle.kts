// Top-level build file of the Kotlin Multiplatform demo of mars-rs.
//
// One module, and the whole point of it is the two halves below: `commonMain`,
// which every platform compiles, and `nativeMain`, which is the one that owns
// `main`. An app that ships on a phone writes the same `commonMain` and puts
// the platform's own entry point beside it.

plugins {
    alias(libs.plugins.kotlin.multiplatform)
}

kotlin {
    // The targets a demo can build *and run* with nothing but a JDK: macOS on
    // Apple silicon, and Linux on x86-64. An app adds the ones it ships on from
    // the same one line each — `androidTarget()`, `iosArm64()`,
    // `iosSimulatorArm64()`, `watchosArm64()`, `mingwX64()` — and every one of
    // them compiles the same `commonMain` below.
    //
    // `macosArm64` and `linuxX64` are executable targets here and library
    // targets in an app: `binaries { executable { ... } }` is what turns the
    // compiled module into a program Gradle can run.
    macosArm64 {
        binaries {
            executable {
                entryPoint = "io.github.orangeboychen.marsrs.demo.main"
            }
        }
    }

    linuxX64 {
        binaries {
            executable {
                entryPoint = "io.github.orangeboychen.marsrs.demo.main"
            }
        }
    }

    sourceSets {
        commonMain {
            dependencies {
                // The logging half of the port, in common code: on Android it
                // reaches the core over JNI and everywhere else over the C ABI,
                // and neither is visible from here — which is the whole point of
                // taking it from `commonMain`.
                implementation(libs.marsrs.xlog)
            }
        }
    }
}
