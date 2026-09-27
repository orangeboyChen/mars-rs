package io.github.orangeboychen.marsrs

/**
 * The surface of the whole port, under the package of the whole port: the names
 * an app that depends on `marsrs-kmp` writes, as opposed to the ones an app
 * that depends on `marsrs-xlog-kmp` writes.
 *
 * Every one of them is xlog's today, and each is a `typealias` rather than a
 * second declaration, because `mars-ffi` is an xlog C ABI: xlog is all a
 * Kotlin/Native target can reach, so `marsrs-kmp` is `marsrs-xlog-kmp` under
 * another name and no more. When STN and SDT join the C ABI, their Kotlin is
 * declared here beside these — and an app that took `marsrs-kmp` picks it up
 * without a rename, which is the only reason this module exists.
 *
 * A module with no declaration of its own cannot be published, by the way:
 * Kotlin makes no klib out of no source, and a publication with no klib in it
 * is a publication Gradle refuses to write. That, and not the typealiases, is
 * what this file is really for.
 */

public typealias Xlog = io.github.orangeboychen.marsrs.xlog.Xlog

public typealias XlogConfig = io.github.orangeboychen.marsrs.xlog.XlogConfig

public typealias LogLevel = io.github.orangeboychen.marsrs.xlog.LogLevel

public typealias AppenderMode = io.github.orangeboychen.marsrs.xlog.AppenderMode

public typealias CompressMode = io.github.orangeboychen.marsrs.xlog.CompressMode

public typealias Log = io.github.orangeboychen.marsrs.xlog.Log
