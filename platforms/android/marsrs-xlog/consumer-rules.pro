# The rules an app's R8 has to have, carried in the AAR as `proguard.txt`.
#
# `marsrs-jni` and this Kotlin are two halves of one binary interface, and the
# interface is spelled in names: the symbol a native is looked up under
# (`Java_io_github_orangeboychen_marsrs_xlog_Xlog_write`), and the member
# `GetFieldID` asks for under the name the Rust spells. R8 sees none of it — a
# native has no caller it can find but JNI, and a field only the Rust reads is
# a field nothing in the app reads.
#
# So an app that builds its release with `minifyEnabled true` — which is what a
# release build is — gets an AAR that links and loads and then dies on the
# first write: `UnsatisfiedLinkError` where R8 renamed `Xlog` or one of its
# natives, `NoSuchFieldError` where it renamed `logdir` or `level` out from
# under `GetFieldID`.
#
# Two things about the rules below are deliberate:
#
# * `{ *; }` is what keeps the members. A bare `-keep class X` keeps the class
#   and lets R8 drop every member of it, which is the same breakage under a
#   name that looks fixed.
# * A whole class is kept and not only the members JNI reaches. The name of a
#   class is part of a JNI *signature* as well —
#   `(Lio/github/orangeboychen/marsrs/xlog/Xlog$XLogConfig;)J` is what
#   `newXlogInstance` is looked up under — so an `XLogConfig` that shrank into
#   `Xlog$a` is a `newXlogInstance` `GetMethodID` cannot find either.
# * `allowoptimization` is what R8 is still free to do. It rewrites bodies and
#   not names, and a name is the whole of what JNI asks about.
#
# This is `platforms/android/marsrs`'s file with everything but xlog left
# out, the same way this module is that module with everything but xlog left
# out.

## 1. Kotlin -> native
##
## Every `external` of this AAR is one `Java_io_github_orangeboychen_marsrs_xlog_Xlog_*`
## symbol of `libmarsrsxlog.so`.
-keepclasseswithmembernames class io.github.orangeboychen.marsrs.xlog.** {
    native <methods>;
}

## 2. native -> Kotlin
##
## What `marsrs-jni` asks for by name: the `@JvmField`s of `XLogConfig` are the
## ones its `config_from_java` reads, and the ones of `XLoggerInfo` the ones
## `logWrite` reads.
-keep,allowoptimization class io.github.orangeboychen.marsrs.xlog.Xlog { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.xlog.Xlog$XLogConfig { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.xlog.Xlog$XLoggerInfo { *; }
