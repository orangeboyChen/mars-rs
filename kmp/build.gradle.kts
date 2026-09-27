// Top-level build file of the Kotlin Multiplatform packaging of mars-rs.
//
// Two modules over the same native library: `mars-xlog` is the Kotlin face of
// the xlog half of the port — the same API on every platform, over the JNI
// bridge on Android and over the C ABI of `mars-ffi` everywhere else — and
// `mars-core` is the whole port over it, the pair the C++ project publishes as
// `mars-core` and `mars-xlog` and the pair `Package.swift` exposes as `MarsRS`
// and `MarsRSXlog`.

plugins {
    // Declared here and applied in the module, so that the version of each
    // plugin is the one thing there is to bump.
    //
    // 8.x and not the 9.x `../android` uses: Android Gradle Plugin 9 compiles
    // Kotlin itself and registers the `kotlin` extension the Kotlin plugin then
    // collides with (issuetracker.google.com/438678642), which is a plugin a
    // Kotlin Multiplatform module cannot do without. 8.13 is the newest version
    // of the line that still leaves `kotlin` to Kotlin — and with it
    // `com.android.library`, and with *that* the `jniLibs` of an AAR, which is
    // where `libmarsxlog.so` has to end up for an app to load it.
    id("com.android.library") version "8.13.2" apply false
    // 2.2 and not 2.3: Kotlin 2.3 marks `androidTarget` — the only way to have
    // an Android target under this plugin — as an error, on the grounds that it
    // does not work with Android Gradle Plugin 9. It does not, and that is why
    // the plugin above is 8.13: 2.3 is right about AGP 9, and AGP 9's
    // replacement, `com.android.kotlin.multiplatform.library`, is the plugin
    // that registers the `kotlin` extension 2.3's own Kotlin plugin then
    // collides with. One of the two has to give, and it is the newer Kotlin.
    id("org.jetbrains.kotlin.multiplatform") version "2.2.20" apply false
}

tasks.register<Delete>("clean") {
    delete(rootProject.layout.buildDirectory)
}
