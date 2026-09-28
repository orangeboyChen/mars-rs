# The rules an app's R8 has to have for the net half, carried in the Android AAR
# of this module as `proguard.txt`.
#
# `src/androidMain` and `marsrs-jni` are two halves of one binary interface, and
# the interface is spelled in names — the same reason `:marsrs-xlog` has a file
# of its own. What differs is *which* names: the logger's are `xlog.*`, and the
# rules that keep them reach a consumer through the `api` dependency
# `commonMain` takes on `:marsrs-xlog`, because consumer rules travel with the
# dependency that carries them. The net half's are these, and nothing else keeps
# them: an app that builds its release with `minifyEnabled true` — which is what
# a release build is — gets an AAR that links and loads and then dies inside the
# first task. `UnsatisfiedLinkError` where R8 renamed `StnLogic.startTask`,
# `NoSuchMethodError` where it renamed the private `onDnsQuery` a probe is asked
# through, `NoSuchFieldError` where it renamed `Task.taskID` out from under the
# `GetFieldID` that reads it.
#
# Two things about the rules below are deliberate:
#
# * `{ *; }` is what keeps the members. A bare `-keep class X` keeps the class
#   and lets R8 drop every member of it, which is the same breakage under a name
#   that looks fixed.
# * `allowoptimization` is what R8 is still free to do: it rewrites bodies and
#   not names, and a name is the whole of what JNI asks about.

## 1. Kotlin -> native
##
## Every `external` of `src/androidMain` is one
## `Java_io_github_orangeboychen_marsrs_{stn,sdt}_*` symbol of
## `libmarsrsxlog.so`: `startTask` and its siblings of `stn.StnLogic`,
## `startActiveCheck` and its siblings of `sdt.SdtLogic`.
-keepclasseswithmembernames class io.github.orangeboychen.marsrs.stn.** {
    native <methods>;
}
-keepclasseswithmembernames class io.github.orangeboychen.marsrs.sdt.** {
    native <methods>;
}

## 2. native -> Kotlin
##
## What `marsrs-jni` asks for by name.

# `stn_c2java.rs`: the `StnLogic` the eighteen questions are asked of — the
# thirteen private statics at the end of `StnLogic.android.kt`, which is what a
# `GetStaticMethodID` of `req2Buf` and the rest resolves — and what three of the
# questions are handed: the task the app started, whose `taskID`, `cmdID`, `cgi`,
# `shortLinkHostList`, `headers` and company `GetFieldID` reads off the object
# by name, and the two profiles a run is reported in. `Task` is a class of
# `commonMain` in its own right, and not a nested `StnLogic$Task`.
-keep,allowoptimization class io.github.orangeboychen.marsrs.stn.StnLogic { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.stn.Task { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.stn.CgiProfile { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.stn.DnsProfile { *; }

# `sdt.rs`: the run's report and its four probes, called on `SdtLogic`'s own
# statics while a run is in flight, and `SdtLogic$Answer` — the class every one
# of the four descriptors names, whose `@JvmField`s the bridge reads.
-keep,allowoptimization class io.github.orangeboychen.marsrs.sdt.SdtLogic { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.sdt.SdtLogic$Answer { *; }
