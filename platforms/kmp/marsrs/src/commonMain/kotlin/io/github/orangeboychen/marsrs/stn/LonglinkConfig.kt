package io.github.orangeboychen.marsrs.stn

/**
 * What a long link of the app's own is made from: the `LonglinkConfig` of
 * `mars/stn/stn.h`, which the C++'s Java does not declare.
 *
 * A link the app names is a second long link beside the one the addresses the
 * setters handed to the net source are for, and [StnLogic.createLonglink] is how
 * it is made. The three calls that take a [name] are the whole of it: what the
 * C++ puts under "Support multi longlinks for mars", which its own Java carries
 * none of.
 *
 * An empty [hostList] is "the hosts the app set", an empty [group] is the
 * long-link group, and a [linkType] of `0` is `Task.E_LONG` — the two defaults a
 * config the app left alone is filled with.
 *
 * The fields are what the two `actual`s read: the JNI bridge of `crates/marsrs-jni`
 * reads them out of the object by name, and the Kotlin/Native one copies them into
 * a `MarsStnLonglinkConfig` of the C ABI.
 */
public class LonglinkConfig(
    /** The name every other call that takes one asks with. */
    public var name: String? = null,
    /** The hosts the link goes out on. */
    public var hostList: List<String>? = null,
    /** `false` leaves the reconnecting to a task. */
    public var isKeepAlive: Boolean = false,
    /** Which links share a reconnect. */
    public var group: String? = null,
    /** Whether this is the link whose status the app is told about. */
    public var isMain: Boolean = false,
    /** One of the `Task.E_*`; `0` is `Task.E_LONG`. */
    public var linkType: Int = 0,
    /** Whether the link is a TLS one. */
    public var needTls: Boolean = true
)
