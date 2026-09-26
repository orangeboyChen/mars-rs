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
 * builds. This one is Rust and there is one, `marsxlog`: the `crate-name` of
 * `mars-jni`, which carries xlog *and* STN *and* SDT, and needs no
 * `libc++_shared.so` beside it.
 *
 * It is an `object` and not a class of statics, but the members are the ones an
 * app calls from Java as `Mars.init(...)`, so they are `@JvmStatic`.
 */
object Mars {

    @JvmStatic
    fun loadDefaultMarsLibrary() {
        try {
            System.loadLibrary("marsxlog")
        } catch (e: Throwable) {
            android.util.Log.e("mars.Mars", "", e)
        }
    }

    @Volatile
    private var hasInitialized = false

    /**
     * Initializes the platform callbacks, and has to be called before `onCreate`: every method of
     * C2Java takes what it asks about from the context that [PlatformComm.init] leaves behind.
     */
    @JvmStatic
    fun init(context: Context, handler: Handler) {
        PlatformComm.init(context, handler)
        hasInitialized = true
    }

    /**
     * Called when the app starts: the first startup has to go through [init] first, and every one
     * after it goes through [BaseEvent.onCreate].
     */
    @JvmStatic
    fun onCreate(isFirstStartup: Boolean) {
        if (isFirstStartup && hasInitialized) {
            BaseEvent.onCreate()
        } else if (!isFirstStartup) {
            BaseEvent.onCreate()
        } else {
            error(
                "Mars.init must be executed before Mars.onCreate when the app starts for the first time."
            )
        }
    }

    /**
     * Destroys the components when the app exits
     */
    @JvmStatic
    fun onDestroy() {
        BaseEvent.onDestroy()
    }
}
