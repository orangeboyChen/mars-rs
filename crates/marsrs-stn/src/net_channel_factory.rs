//! `mars/stn/src/net_channel_factory.cc` — the channels a task is made on, and
//! the app's own way of making them.
//!
//! The C++ has two namespaces of two function pointers each, `Create` and
//! `Destory`, which are weak symbols: an app that links its own object file
//! gets its own channel instead of mars's. A host here does the same thing by
//! replacing a hook, which is what [`ChannelFactory`] is: two of them, one per
//! kind of channel, and mars's own channel behind each until it is replaced.
//!
//! `Destory` is not ported: it is `delete` in the C++, and a value in Rust needs
//! nothing to drop it.

use crate::{LongLink, LonglinkConfig, ShortLink, Task};

/// `LongLinkChannelFactory::Create` — the long link for a config, which is
/// `new LongLink(…, _config, gDefaultLongLinkEncoder)` in the C++.
pub type CreateLongLink = dyn FnMut(&LonglinkConfig) -> LongLink + Send;
/// `ShortLinkChannelFactory::Create` — the short link for a task, which is
/// `new ShortLink(…, _task, _config.use_proxy)` in the C++. The
/// `ShortlinkConfig` the C++ takes has nothing else the port's short link
/// needs, so what is handed over is the one field of it that matters.
pub type CreateShortLink = dyn FnMut(Task, bool) -> ShortLink + Send;

/// `LongLinkChannelFactory` and `ShortLinkChannelFactory` as one value.
pub struct ChannelFactory {
    longlink: Box<CreateLongLink>,
    shortlink: Box<CreateShortLink>,
}

impl ChannelFactory {
    /// The two hooks mars ships: a [`LongLink`] and a [`ShortLink`], made the
    /// way `net_channel_factory.cc` makes them.
    pub fn new() -> Self {
        Self {
            longlink: Box::new(|config| LongLink::new(config.clone())),
            shortlink: Box::new(ShortLink::new),
        }
    }

    /// `LongLinkChannelFactory::Create = …` — the app's own long link.
    pub fn set_create_longlink(
        &mut self,
        create: impl FnMut(&LonglinkConfig) -> LongLink + Send + 'static,
    ) {
        self.longlink = Box::new(create);
    }

    /// `ShortLinkChannelFactory::Create = …` — the app's own short link.
    pub fn set_create_shortlink(
        &mut self,
        create: impl FnMut(Task, bool) -> ShortLink + Send + 'static,
    ) {
        self.shortlink = Box::new(create);
    }

    /// `LongLinkChannelFactory::Create(_context, _messagequeueid, _netsource,
    /// _config)`.
    pub fn create_longlink(&mut self, config: &LonglinkConfig) -> LongLink {
        (self.longlink)(config)
    }

    /// `ShortLinkChannelFactory::Create(_context, _messagequeueid, _netsource,
    /// _task, _config)`.
    pub fn create_shortlink(&mut self, task: Task, use_proxy: bool) -> ShortLink {
        (self.shortlink)(task, use_proxy)
    }
}

impl Default for ChannelFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ChannelFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelFactory").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_channels_mars_ships_are_a_long_link_and_a_short_link() {
        let mut factory = ChannelFactory::new();
        let config = LonglinkConfig::new("long.weixin.qq.com");
        let link = factory.create_longlink(&config);
        assert_eq!(link.config().name, "long.weixin.qq.com");

        let mut task = Task::new(7, 12);
        task.cgi = "/cgi".to_string();
        let short = factory.create_shortlink(task, true);
        assert_eq!(short.task().cgi, "/cgi");
    }

    #[test]
    fn a_channel_the_app_makes_itself_is_the_one_that_comes_out() {
        let mut factory = ChannelFactory::new();
        factory.set_create_longlink(|config| {
            let mut config = config.clone();
            config.name = "the app's own".to_string();
            LongLink::new(config)
        });
        factory.set_create_shortlink(|task, _use_proxy| {
            let mut short = ShortLink::new(task, false);
            short.set_sent_count(3);
            short
        });

        let link = factory.create_longlink(&LonglinkConfig::new("long.weixin.qq.com"));
        assert_eq!(link.config().name, "the app's own", "not mars's channel");

        let task = Task::new(7, 12);
        assert_eq!(factory.create_shortlink(task, true).sent_count(), 3);
    }

    #[test]
    fn the_factory_is_the_same_one_the_next_channel_comes_from() {
        let mut factory = ChannelFactory::new();
        let config = LonglinkConfig::new("long.weixin.qq.com");
        let _ = factory.create_longlink(&config);
        // the config is borrowed for the one call, not kept
        assert_eq!(factory.create_longlink(&config).config().name, config.name);
    }

    #[test]
    fn the_default_factory_is_mars_s_own() {
        let mut factory = ChannelFactory::default();
        assert!(format!("{factory:?}").contains("ChannelFactory"));
        let config = LonglinkConfig::new("long.weixin.qq.com");
        assert_eq!(
            factory.create_longlink(&config).config().group,
            config.group
        );
    }
}
