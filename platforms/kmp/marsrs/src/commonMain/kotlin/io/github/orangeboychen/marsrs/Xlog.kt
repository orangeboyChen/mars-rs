package io.github.orangeboychen.marsrs

/**
 * The surface of the whole port, under the package of the whole port: the names
 * an app that depends on `marsrs-kmp` writes, as opposed to the ones an app
 * that depends on `xlog-kmp` writes.
 *
 * Every one of them is xlog's, and each is a `typealias` rather than a second
 * declaration: these are the names `marsrs-kmp` published first, while xlog was
 * all the C ABI carried, and they stay at the top of the package so that an app
 * that wrote `Xlog` keeps writing `Xlog`. STN and SDT joined the C ABI after
 * them and are declared in `stn` and `sdt` below instead — nothing of theirs
 * was ever published from here, so nothing of theirs is aliased.
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
