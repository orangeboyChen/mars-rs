#import <Flutter/Flutter.h>

NS_ASSUME_NONNULL_BEGIN

/// The iOS half of `marsrs`: the fourteen methods of the plugin's
/// channel, each of them a straight call of a `mars_xlog_*` symbol.
@interface XlogPlugin : NSObject <FlutterPlugin>

@end

NS_ASSUME_NONNULL_END
