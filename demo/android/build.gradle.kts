// Top-level build file of the Android demo of mars-rs.
//
// One thing lives here: the version of the Android Gradle Plugin, declared once
// and applied in the module, so that the version is the single line there is to
// bump. Everything else belongs to `app/build.gradle.kts`.

plugins {
    // 9.4.1 is the newest stable AGP; 9.5.0 is alpha. It brings its own Kotlin
    // compiler, so no `org.jetbrains.kotlin.android` is declared anywhere in
    // this build — AGP 9 refuses the plugin, having taken the job over
    // (issuetracker.google.com/438678642). That is also why the module can set
    // `kotlin { compilerOptions { jvmTarget } }` with no Kotlin plugin applied.
    alias(libs.plugins.android.application) apply false
}

tasks.register<Delete>("clean") {
    delete(rootProject.layout.buildDirectory)
}
