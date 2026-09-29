package io.github.orangeboychen.marsrs

/**
 * `BaseEvent` of `com/tencent/mars/BaseEvent.java`: how the app tells the port
 * what happened to it, which is what `StnLogic` and `SdtLogic` are not — those
 * two are what an app asks the port to do, and this is what the app reports.
 *
 * One `expect`, two `actual`s, the way the two of those are: over the C ABI of
 * `crates/marsrs-ffi` (`mars_stn.h`) on every Kotlin/Native target, and over the
 * JNI bridge of `crates/marsrs-jni` on Android. The name is the one the Android
 * AAR publishes, so a shared module that moves between `marsrs-kmp` and
 * `marsrs` has nothing to rename when it does.
 *
 * Two of the seven `BaseEvent.java` declares are here, because they are the two
 * the net core asks about while it runs a task: [onForeground] is what it reads
 * before it wakes a long link that is down, and [onNetworkChange] is what tells
 * it that the ip the last connect landed on is one the network no longer
 * answers. An app that calls neither gets the C++'s `ActiveLogic` as it is made
 * — not in front, so nothing is woken for a task.
 *
 * The five that are not here are five the port does without being told: the net
 * core is made the first time anything asks for it and dropped when the process
 * ends, the encoder version is `StnLogic.resetAndInitEncoderVersion`'s, and what
 * the two crash calls do is close the appender, which is `Xlog.close`.
 */
public expect object BaseEvent {
    /**
     * `ActiveLogic::OnForeground` — the app came to the front, or left it, which
     * is what the net core asks before it wakes a long link that is down for a
     * task, and what the anti-avalanche check and the timing sync are told.
     *
     * A move in either direction makes the app active again, and ten minutes in
     * the background end that — the C++ counts them on an alarm of its own, and
     * here they are counted by the loop that calls `StnLogic.runPending`.
     */
    public fun onForeground(isForeground: Boolean)

    /**
     * `GetSignalOnNetworkChange` — the network under the app changed.
     *
     * The C++ clears the network information it cached before it fires this; the
     * port asks the platform on every read instead, so there is no cache to
     * clear.
     */
    public fun onNetworkChange()
}
