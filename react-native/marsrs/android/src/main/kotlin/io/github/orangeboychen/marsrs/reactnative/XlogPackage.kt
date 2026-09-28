// The `ReactPackage` of the Android half of `marsrs-react-native`.
//
// Autolinking finds this class by the `ReactPackage` it is and the
// `react-native` directory it is in: it is what puts `XlogModule` in React
// Native's hands, and nothing in the app has to name either of them.
//
// A `BaseReactPackage` and not a `ReactPackage`: `BaseReactPackage` is the one
// that answers `getModule`, which is what React Native asks a TurboModule for,
// and the `ReactModuleInfo` below is what tells it this module is one. A package
// that only answered `createNativeModules` would be handed to the bridge, and
// every call would be a promise again — which is the whole of what
// `src/index.ts` is not.

package io.github.orangeboychen.marsrs.reactnative

import com.facebook.react.BaseReactPackage
import com.facebook.react.bridge.NativeModule
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.module.model.ReactModuleInfo
import com.facebook.react.module.model.ReactModuleInfoProvider

/** The package of `Xlog`. */
class XlogPackage : BaseReactPackage() {

    override fun getModule(name: String, reactContext: ReactApplicationContext): NativeModule? =
        if (name == XlogModule.NAME) XlogModule(reactContext) else null

    /**
     * The one module this package has, and what React Native asks it about
     * before it has an instance: its class, so that one can be made without
     * asking, and `isTurboModule`, which is what sends the JSI after it rather
     * than the bridge.
     */
    override fun getReactModuleInfoProvider(): ReactModuleInfoProvider = ReactModuleInfoProvider {
        mapOf(
            XlogModule.NAME to ReactModuleInfo(
                XlogModule.NAME,
                XlogModule::class.java.name,
                CAN_OVERRIDE_EXISTING_MODULE,
                NEEDS_EAGER_INIT,
                HAS_CONSTANTS,
                IS_CXX_MODULE,
                IS_TURBO_MODULE
            )
        )
    }

    private companion object {
        /** The module is the only one of its name an app has. */
        const val CAN_OVERRIDE_EXISTING_MODULE = false

        /** Nothing of the module is read before it is asked for. */
        const val NEEDS_EAGER_INIT = false

        /** The module carries no `constantsToExport`: a logger has none. */
        const val HAS_CONSTANTS = false

        /** Kotlin over a JNI bridge, and not a C++ module of React Native's
         * own. */
        const val IS_CXX_MODULE = false

        /** What makes `src/index.ts` synchronous. */
        const val IS_TURBO_MODULE = true
    }
}
