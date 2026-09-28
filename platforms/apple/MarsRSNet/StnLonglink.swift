// `StnLonglinkConfig` — what a long link of the app's own is made from, which is
// `LonglinkConfig` of `mars/stn/stn.h` — and the way across: the C struct
// `mars_stn_create_longlink` reads.
//
// The C++ puts `CreateLonglink_ext`, `DestroyLonglink_ext` and
// `MarkMainLonglink_ext` under "Support multi longlinks for mars", and declares
// no Java for any of them, so there is no name an app migrating from that API
// already writes: the three are this port's, named the way `MarsStn` names the
// rest.

import Foundation

import MarsRSNetFFI

/// What a long link of the app's own is made from: `LonglinkConfig` of
/// `mars/stn/stn.h`, which `MarsStn` names `MarsStn.LonglinkConfig`.
///
/// An empty `hostList` is "the hosts the app set", an empty `group` is the
/// long-link group and a `linkType` of `0` is the long link — the two defaults
/// the C ABI fills a zeroed struct with, and so the ones a config the app left
/// alone is made with.
public struct StnLonglinkConfig {
    /// The name every other call that takes one asks with.
    public var name: String
    /// The hosts the link goes out on; empty is "the hosts the app set".
    public var hostList: [String]
    /// `false` leaves the reconnecting to a task.
    public var isKeepAlive: Bool
    /// Which links share a reconnect; empty is the long-link group.
    public var group: String
    /// Whether this is the link whose status the app is told about.
    public var isMain: Bool
    /// One of the `Task::CHANNEL_*`; `0` is the long link.
    public var linkType: Int32
    /// Whether the link is a TLS one.
    public var needTLS: Bool

    /// A long link of that name, and the rest at the defaults a zeroed C struct
    /// is filled with.
    public init(
        name: String,
        hostList: [String] = [],
        isKeepAlive: Bool = false,
        group: String = "",
        isMain: Bool = false,
        linkType: Int32 = 0,
        needTLS: Bool = true
    ) {
        self.name = name
        self.hostList = hostList
        self.isKeepAlive = isKeepAlive
        self.group = group
        self.isMain = isMain
        self.linkType = linkType
        self.needTLS = needTLS
    }
}

/// The C struct `config` becomes, lent for the length of a call: the two strings
/// and the host list are pointers into memory `body` owns, so the struct is made
/// inside it and not before.
internal func withLonglinkConfig<R>(
    _ config: StnLonglinkConfig,
    body: (UnsafePointer<MarsStnLonglinkConfig>) -> R
) -> R {
    withCStrings([config.name, config.group]) { strings in
        withStrings(config.hostList) { hosts in
            var value = MarsStnLonglinkConfig(
                name: strings[0],
                host_list: MarsStnStrings(items: hosts, count: UInt32(config.hostList.count)),
                is_keep_alive: config.isKeepAlive ? 1 : 0,
                group: strings[1],
                is_main: config.isMain ? 1 : 0,
                link_type: config.linkType,
                need_tls: config.needTLS ? 1 : 0
            )
            return withUnsafePointer(to: &value, body)
        }
    }
}
