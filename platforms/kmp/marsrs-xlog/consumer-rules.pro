# The rules an app's R8 has to have, carried in the Android AAR of this module
# as `proguard.txt`.
#
# `marsrs-jni` and the `androidMain` below are two halves of one binary
# interface, and the interface is spelled in names: the symbol a native is
# looked up under (`Java_io_github_orangeboychen_marsrs_xlog_Xlog_write`), and
# the field `GetFieldID` asks for under the name the Rust spells. R8 sees none
# of it — a native has no caller it can find but JNI, and a field only the Rust
# reads is a field nothing in the app reads.
#
# So an app that builds its release with `minifyEnabled true` — which is what a
# release build is — gets a klib that links and loads and then dies on the
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
#   `(Lio/github/orangeboychen/marsrs/xlog/XLogConfigJni;)J` is what
#   `newXlogInstance` is looked up under — so an `XLogConfigJni` that shrank
#   into a class of another name is a `newXlogInstance` `GetMethodID` cannot
#   find either.
# * `allowoptimization` is what R8 is still free to do. It rewrites bodies and
#   not names, and a name is the whole of what JNI asks about.
#
# `platforms/kmp/marsrs` needs no rules for the logger: it is this module's API
# re-exported, and consumer rules travel with the dependency that carries them.
# It has a file of its own all the same — for the net half, which is its own
# code over `marsrs-jni` and not this module's.

## 1. Kotlin -> native
##
## Every `external` of the Android target is one
## `Java_io_github_orangeboychen_marsrs_xlog_Xlog_*` symbol of
## `libmarsrsxlog.so`: `write` is a `@JvmStatic` of the companion because
## `marsrs-jni` takes the handle as an argument and not as a receiver, and the
## rest are instance natives of `Xlog` itself.
-keepclasseswithmembernames class io.github.orangeboychen.marsrs.xlog.** {
    native <methods>;
}

## 2. native -> Kotlin
##
## What `marsrs-jni` asks for by name: the `@JvmField`s of `XLogConfigJni` are
## the ones its `config_from_java` reads.
-keep,allowoptimization class io.github.orangeboychen.marsrs.xlog.Xlog { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.xlog.XLogConfigJni { *; }
