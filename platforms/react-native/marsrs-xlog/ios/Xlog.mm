// The one file of the iOS half of `marsrs-react-native-xlog` that Swift cannot
// be, and why: a TurboModule is made of a factory that answers a
// `std::shared_ptr<facebook::react::TurboModule>`, and `NativeXlogSpecJSI` — the
// C++ class codegen wrote out of `src/NativeXlog.ts` — is a class of templates
// and jsi types. Neither has a Swift spelling: Swift's C++ interop stops at
// templates and at the standard library. So what is here is the three things
// React Native asks a module for and cannot read out of `Xlog.swift`, and the
// fourteen methods are there and not here.
//
//   * `RCT_EXTERN_MODULE` registers the Swift class, which Swift cannot do for
//     itself — registration is a constructor the runtime runs, and a Swift class
//     has no `+load`. It is also what gives the class its `moduleName`, which is
//     the name `src/index.ts` reads it out of `TurboModuleRegistry` by.
//   * `RCTTurboModule` is adopted here, in a category, and not in the Swift:
//     its one method is the C++ factory above, so a Swift class can neither name
//     it nor be asked about it. `conformsToProtocol:` still answers `YES`, which
//     is what `RCTTurboModuleManager` asks before it hands a module to JSI.
//   * `methodQueue` answers `RCTJSThread`, the queue that is no queue: it is
//     what makes a TurboModule's methods run on the JS thread, so a call is made
//     where it is asked for and a method that answers a value answers it before
//     it returns. Without it the eleven would be dispatched to a queue of the
//     module's own and every answer would arrive as `undefined`.
//
// Objective-C++, and not Objective-C, because of the one `#include` a Swift
// class cannot make: `RCTNativeXlogSpec.h` opens with
// `#error This file must be compiled as Obj-C++`, and with good reason — it
// declares a C++ class.

#import <React/RCTBridgeModule.h>
#import <React/RCTJSThread.h>
#import <React/RCTTurboModule.h>

// What codegen wrote out of `src/NativeXlog.ts`: the protocol `NativeXlogSpec`
// and the C++ class `NativeXlogSpecJSI`, in one header. React Native names the
// file, and renamed it once — `RCTNativeXlogSpec.h` since 0.74,
// `NativeXlogSpec.h` before — so both spellings are asked for.
#if __has_include(<React-Codegen/RCTNativeXlogSpec.h>)
#import <React-Codegen/RCTNativeXlogSpec.h>
#else
#import <React-Codegen/NativeXlogSpec.h>
#endif

// `Xlog`, the Swift class, registered under the name `src/index.ts` reads it by.
@interface RCT_EXTERN_MODULE(Xlog, NSObject)
@end

@interface Xlog (XlogTurboModule) <RCTTurboModule>
@end

@implementation Xlog (XlogTurboModule)

- (dispatch_queue_t)methodQueue
{
  return RCTJSThread;
}

- (std::shared_ptr<facebook::react::TurboModule>)getTurboModule:
    (const facebook::react::ObjCTurboModule::InitParams &)params
{
  return std::make_shared<facebook::react::NativeXlogSpecJSI>(params);
}

@end
