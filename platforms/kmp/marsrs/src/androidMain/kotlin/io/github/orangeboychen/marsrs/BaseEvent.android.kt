package io.github.orangeboychen.marsrs

/**
 * The Android `actual`: the two `Java_io_github_orangeboychen_marsrs_BaseEvent_*`
 * symbols of `marsrs-jni`, which is why the object is this class in this package
 * and why both are `@JvmStatic` — the same two reasons `StnLogic` and `Xlog` are
 * what they are in this module. The class is the one the Android AAR declares
 * under the same name, so an app that moves between the two has nothing to
 * rename either.
 */
public actual object BaseEvent {
    init {
        // `marsrsxlog`, the `crate-name` of `marsrs-jni`: loaded before the first
        // symbol of it is called, and loading it twice is nothing — which this
        // one is, `StnLogic` and `Xlog` being the classes an app touches first.
        System.loadLibrary(LIBRARY)
    }

    @JvmStatic
    public actual external fun onForeground(isForeground: Boolean)

    @JvmStatic
    public actual external fun onNetworkChange()

    /** `marsrsxlog` — the `crate-name` of `marsrs-jni`, i.e. the library every symbol above lives in. */
    private const val LIBRARY = "marsrsxlog"
}
