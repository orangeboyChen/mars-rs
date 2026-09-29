package io.github.orangeboychen.marsrs

import android.content.Context
import android.os.Handler
import io.github.orangeboychen.marsrs.comm.PlatformComm

/**
 * The entry point of the port on Android: it loads the one native library and
 * hands the process' lifecycle over to [BaseEvent].
 *
 * What the C++ project's `Mars.java` loads is three libraries — `c++_shared`,
 * `marsxlog` and `marsstn` — because it is C++ and its two `.so` files are two
 * builds. This one is Rust and there is one, `marsrsxlog`: the `crate-name` of
 * `marsrs-jni`, which carries xlog *and* STN *and* SDT, and needs no
 * `libc++_shared.so` beside it.
 *
 * It is an `object` and not a class of statics, but the members are the ones an
 * app calls from Java as `Mars.init(...)`, so they are `@JvmStatic`.
 */
object Mars {

    /** `marsrsxlog` — the `crate-name` of `marsrs-jni`, and the one library every `external` of this AAR lives in. */
    private const val LIBRARY = "marsrsxlog"

    /**
     * Whether `libmarsrsxlog.so` is in this process: what [loadDefaultMarsLibrary]
     * sets, and what [requireLibrary] asks before a symbol is reached. It is
     * public because [loadDefaultMarsLibrary] cannot be the only way an app
     * hears the answer — that one logs a failure and goes on.
     *
     * A load that failed used to be a log line and nothing else, and the failure
     * then came out of whichever `external` an app called first — thrown on the
     * thread the port was running on, as an `UnsatisfiedLinkError` naming a
     * symbol and not the load that never happened.
     */
    @Volatile
    var libraryLoaded = false
        private set

    @JvmStatic
    fun loadDefaultMarsLibrary() {
        try {
            System.loadLibrary(LIBRARY)
            libraryLoaded = true
        } catch (e: Throwable) {
            // Logged and not rethrown: this is called from the `init` of
            // `StnLogic` and of `SdtLogic`, and an exception out of the `init`
            // of a class leaves it one the app can never touch again. What names
            // the library instead is [requireLibrary], on the call that reaches
            // a symbol.
            android.util.Log.e("mars.Mars", "System.loadLibrary(\"$LIBRARY\") failed", e)
        }
    }

    /**
     * `libmarsrsxlog.so`, or an `IllegalStateException` naming it: what
     * [onCreate] and [onDestroy] ask before [BaseEvent] reaches a symbol, so
     * that a process the library is not in says so on the call an app made
     * rather than inside one.
     *
     * Those two are the entry points of this object, and the ones its own
     * `external`s are behind. `StnLogic`, `SdtLogic` and the `BaseEvent` a
     * broadcast reaches are not guarded this way — their `init` has to log a
     * failed load and go on, because an exception out of it is a class the app
     * can never touch again — so a call of theirs with the library missing is
     * still an `UnsatisfiedLinkError` naming the symbol. [libraryLoaded] is
     * what an app asks when it wants that answer before it makes one.
     */
    private fun requireLibrary() {
        if (!libraryLoaded) {
            loadDefaultMarsLibrary()
        }
        check(libraryLoaded) {
            "lib$LIBRARY.so is not loaded: no `external` of io.github.orangeboychen.marsrs can be answered without it"
        }
    }

    @Volatile
    private var hasInitialized = false

    /**
     * Initializes the platform callbacks, and has to be called before `onCreate`: every method of
     * C2Java takes what it asks about from the context that [PlatformComm.init] leaves behind.
     *
     * What is kept is the application context of the one handed over — so an
     * Activity is safe to hand over — and it is kept for the process: the nine
     * questions are answered from it whenever the port asks, which is why
     * [release] is the way an app lets go of it.
     */
    @JvmStatic
    fun init(context: Context, handler: Handler) {
        PlatformComm.init(context, handler)
        hasInitialized = true
    }

    /**
     * Lets go of what [init] kept: the context, the handler and the phone-state
     * listener `NetworkSignalUtil` put on the air. Nothing of `C2Java` answers
     * afterwards, so this is for an app that is done with the port and not one
     * that goes on being asked about the device.
     */
    @JvmStatic
    fun release() {
        PlatformComm.release()
        hasInitialized = false
    }

    /**
     * Called when the app starts: the first startup has to go through [init] first, and every one
     * after it goes through [BaseEvent.onCreate].
     */
    @JvmStatic
    fun onCreate(isFirstStartup: Boolean) {
        if (isFirstStartup && !hasInitialized) {
            error(
                "Mars.init must be executed before Mars.onCreate when the app starts for the first time."
            )
        }
        requireLibrary()
        BaseEvent.onCreate()
    }

    @JvmStatic
    fun onDestroy() {
        requireLibrary()
        BaseEvent.onDestroy()
    }
}
