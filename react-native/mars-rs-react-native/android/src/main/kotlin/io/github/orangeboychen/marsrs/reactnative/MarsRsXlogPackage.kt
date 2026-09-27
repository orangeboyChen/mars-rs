// The `ReactPackage` of the Android half of `mars-rs-react-native`.
//
// Autolinking finds this class by the `ReactPackage` it is and the
// `react-native` directory it is in: it is what puts `MarsRsXlogModule` in
// `NativeModules`, and nothing in the app has to name either of them.

package io.github.orangeboychen.marsrs.reactnative

import com.facebook.react.ReactPackage
import com.facebook.react.bridge.NativeModule
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.uimanager.ViewManager

/** The package of `MarsRsXlog`. */
class MarsRsXlogPackage : ReactPackage {

    override fun createNativeModules(context: ReactApplicationContext) = listOf<NativeModule>(MarsRsXlogModule(context))

    // The module is a logger: it has no view, and RN asks for the list all the
    // same.
    override fun createViewManagers(reactContext: ReactApplicationContext): List<ViewManager<*, *>> = emptyList()
}
