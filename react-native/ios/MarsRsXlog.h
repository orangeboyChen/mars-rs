#import <React/RCTBridgeModule.h>

NS_ASSUME_NONNULL_BEGIN

/// The iOS half of `mars-rs-react-native-xlog`: the six methods of the
/// `MarsRsXlog` native module, each of them a straight call of a `mars_xlog_*`
/// symbol.
@interface MarsRsXlog : NSObject <RCTBridgeModule>

@end

NS_ASSUME_NONNULL_END
