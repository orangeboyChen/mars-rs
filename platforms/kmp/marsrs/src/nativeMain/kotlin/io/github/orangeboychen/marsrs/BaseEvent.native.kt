package io.github.orangeboychen.marsrs

import io.github.orangeboychen.marsrs.net.ffi.mars_stn_on_foreground
import io.github.orangeboychen.marsrs.net.ffi.mars_stn_on_network_change

/**
 * The Kotlin/Native `actual`: the two symbols of `mars_stn.h` the C ABI of
 * `crates/marsrs-ffi` grew for the two things `mars::baseevent` hands every
 * platform, which is the same source set for every native target — iOS, watchOS,
 * tvOS, macOS, Linux and Windows — because the C ABI is the same header on all
 * of them.
 *
 * There is no `BaseEvent` in an ABI of plain functions, so a host of the header
 * calls these itself; this is that call in Kotlin, which is the whole of what an
 * app of a Kotlin Multiplatform project sees of `mars::baseevent`.
 */
public actual object BaseEvent {
    public actual fun onForeground(isForeground: Boolean) {
        mars_stn_on_foreground(if (isForeground) 1 else 0)
    }

    public actual fun onNetworkChange() {
        mars_stn_on_network_change()
    }
}
