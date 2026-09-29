# The rules an app's R8 has to have, carried in the AAR as `proguard.txt`.
#
# `marsrs-jni` and this Kotlin are two halves of one binary interface, and the
# interface is spelled in names: the symbol a native is looked up under
# (`Java_io_github_orangeboychen_marsrs_stn_StnLogic_startTask`), the class
# `find_class` asks for, and the member `GetStaticMethodID` and `GetFieldID`
# ask for under the name the Rust spells. R8 sees none of it — a method only
# JNI calls has no caller it can find, so it is unreachable, and a field only
# the Rust reads is a field nothing in the app reads.
#
# So an app that builds its release with `minifyEnabled true` — which is what
# a release build is — gets an AAR that links and loads and then dies on the
# first call: `UnsatisfiedLinkError` where R8 renamed a class or a native,
# `NoSuchMethodError` and `NoSuchFieldError` where it deleted a callback or
# renamed a field out from under `GetFieldID`.
#
# Two things about the rules below are deliberate:
#
# * `{ *; }` is what keeps the members. A bare `-keep class X` keeps the class
#   and lets R8 drop every member of it, which is the same breakage under a
#   name that looks fixed: measured against R8 9.4.24, the AAR's classes come
#   out of a shrunk build with their own names and nothing inside them.
# * A whole class is kept and not only the members JNI reaches. The name of a
#   class is part of a JNI *signature* as well —
#   `(Lio/github/orangeboychen/marsrs/stn/StnLogic$Task;)V` is what
#   `startTask` is looked up under — so a `Task` that shrank into `StnLogic$a`
#   is a `startTask` `GetMethodID` cannot find either.
# * `allowoptimization` is what R8 is still free to do. It rewrites bodies and
#   not names, and a name is the whole of what JNI asks about.

## 1. Kotlin -> native
##
## Every `external` of this AAR is one `Java_<class>_<method>` symbol of
## `libmarsrsxlog.so`, and JNI resolves it by the names the class file carries:
## `BaseEvent`, `comm.Alarm`, `sdt.SdtLogic`, `stn.StnLogic`, `xlog.Xlog`.
-keepclasseswithmembernames class io.github.orangeboychen.marsrs.** {
    native <methods>;
}

## 2. native -> Kotlin
##
## What `marsrs-jni` asks for by name. The private members of `StnLogic` and
## `PlatformComm$C2Java` are the callbacks the C++ project's own Java declares
## the same way, and the `@JvmField`s of the classes below — `uin`,
## `devicename`, `ssid`, `ispCode`, `taskID`, `headers`, `logdir`, `level` and
## the rest — are the ones its `GetFieldID` spells.

# `app_logic.rs`: the app directory, the account and the device
-keep,allowoptimization class io.github.orangeboychen.marsrs.app.AppLogic { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.app.AppLogic$AccountInfo { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.app.AppLogic$DeviceInfo { *; }

# `platform_comm.rs`: the nine `C2Java` statics, and what three of them answer
-keep,allowoptimization class io.github.orangeboychen.marsrs.comm.PlatformComm { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.comm.PlatformComm$C2Java { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.comm.PlatformComm$WifiInfo { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.comm.PlatformComm$SIMInfo { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.comm.PlatformComm$APNInfo { *; }

# `sdt.rs`: the signal detection results
-keep,allowoptimization class io.github.orangeboychen.marsrs.sdt.SdtLogic { *; }
# The class every one of the four probe descriptors names —
# `(Ljava/lang/String;I)Lio/github/orangeboychen/marsrs/sdt/SdtLogic$Answer;` —
# so an `Answer` R8 renamed is a `GetStaticMethodID` that finds no `onDnsQuery`
# and no `onTcpQuery` at all, and a diagnosis that answers nothing.
-keep,allowoptimization class io.github.orangeboychen.marsrs.sdt.SdtLogic$Answer { *; }
# One entry of the two `CheckIPPorts` a run is started with: `hosts_from_java`
# reads `name`, `hosts` and `ports` off every element of the array it is handed.
-keep,allowoptimization class io.github.orangeboychen.marsrs.sdt.SdtLogic$Link { *; }

# `stn_c2java.rs`: the fifteen `ICallBack` questions, and the task and the
# profile the two halves pass to each other
-keep,allowoptimization class io.github.orangeboychen.marsrs.stn.StnLogic { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.stn.StnLogic$Task { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.stn.StnLogic$CgiProfile { *; }
# What `createLonglink` is handed: `longlink_config_from_java` reads `name`,
# `hostList`, `isKeepAlive`, `group`, `isMain`, `linkType` and `needTls` off it,
# and a field R8 renamed out from under a `GetFieldID` is no error — it is a
# link the app was answered `false` for.
-keep,allowoptimization class io.github.orangeboychen.marsrs.stn.StnLogic$LonglinkConfig { *; }

# `jni_bridge.rs`: what `appenderOpen` and `logWrite` read out of their argument
-keep,allowoptimization class io.github.orangeboychen.marsrs.xlog.Xlog { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.xlog.Xlog$XLogConfig { *; }
-keep,allowoptimization class io.github.orangeboychen.marsrs.xlog.Xlog$XLoggerInfo { *; }
