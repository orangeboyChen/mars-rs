package io.github.orangeboychen.marsrs;

import android.content.Context;
import android.os.Handler;

import io.github.orangeboychen.marsrs.comm.PlatformComm;

/**
 * The entry point of the port on Android: it loads the one native library and
 * hands the process' lifecycle over to {@link BaseEvent}.
 *
 * What the C++ project's `Mars.java` loads is three libraries — `c++_shared`,
 * `marsxlog` and `marsstn` — because it is C++ and its two `.so` files are two
 * builds. This one is Rust and there is one, `marsxlog`: the `crate-name` of
 * `mars-jni`, which carries xlog *and* STN *and* SDT, and needs no
 * `libc++_shared.so` beside it.
 */
public class Mars {

    public static void loadDefaultMarsLibrary() {
        try {
            System.loadLibrary("marsxlog");
        }
        catch (Throwable e) {
            android.util.Log.e("mars.Mars", "", e);
        }
    }

    private static volatile boolean hasInitialized  = false;

    /**
     * 初始化平台回调，必须在 onCreate 之前调用：C2Java 的每一个方法都从
     * {@link PlatformComm#init} 留下的 context 里取它要问的东西。
     */
    public static void init(Context _context, Handler _handler) {
        PlatformComm.init(_context, _handler);
        hasInitialized = true;
    }

    /**
     * APP 启动时调用：首次启动必须先 {@link #init}，之后每一次都走
     * {@link BaseEvent#onCreate}。
     */
    public static void onCreate(boolean isFirstStartup) {
        if (isFirstStartup && hasInitialized) {
            BaseEvent.onCreate();
        }
        else if (!isFirstStartup) {
            BaseEvent.onCreate();
        }
        else {
            throw new IllegalStateException("Mars.init must be executed before Mars.onCreate when the app starts for the first time.");
        }
    }

    /**
     *  APP 退出时销毁组件
     */
    public static void onDestroy() {
        BaseEvent.onDestroy();
    }

}
