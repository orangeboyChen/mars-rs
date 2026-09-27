//! `mars/comm/comm_data.h` — the proxy a connect can go through.
//!
//! `ProxyInfo` is what the C++ hands `ComplexConnect::ConnectImpatient` and
//! `SocketOperator::Connect`: how to reach the proxy, if there is one, and what
//! to log in with. It is a value here, not a bundle of out-parameters, and
//! [`ProxyInfo::is_valid`] is the C++'s question about it.

/// `ProxyType` — how a proxy is talked to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProxyType {
    /// `kProxyNone` — no proxy at all.
    #[default]
    None,
    /// `kProxyHttpTunel`
    HttpTunnel,
    /// `kProxySocks5`
    Socks5,
    /// `kProxyHttp`
    Http,
}

impl ProxyType {
    /// `kProxyNone == type` — whether there is no proxy to go through, which is
    /// the one a `ProxyInfo` is by default.
    pub fn is_none(self) -> bool {
        matches!(self, Self::None)
    }
}

/// `ProxyInfo`.
#[derive(Debug, Clone, Default)]
pub struct ProxyInfo {
    /// `type`
    pub kind: ProxyType,
    /// `host`
    pub host: String,
    /// `ip`
    pub ip: String,
    /// `port`
    pub port: u16,
    /// `username`
    pub username: String,
    /// `password`
    pub password: String,
}

impl ProxyInfo {
    /// `ProxyInfo()` — no proxy.
    pub fn none() -> Self {
        Self::default()
    }

    /// `ProxyInfo(_type, _host, _ip, _port, _username, _password)`.
    pub fn new(
        kind: ProxyType,
        host: impl Into<String>,
        ip: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            host: host.into(),
            ip: ip.into(),
            port,
            username: username.into(),
            password: password.into(),
        }
    }

    /// `IsValid()` — no proxy is valid, and so is one that says where the proxy
    /// is: an ip or a host, and a port.
    pub fn is_valid(&self) -> bool {
        self.kind.is_none() || self.is_address_valid()
    }

    /// `IsAddressValid()` — a proxy that is not [`ProxyType::None`] and says
    /// where it is.
    pub fn is_address_valid(&self) -> bool {
        !self.kind.is_none() && (!self.ip.is_empty() || !self.host.is_empty()) && self.port > 0
    }
}

impl PartialEq for ProxyInfo {
    /// `operator==` of the C++, which is what makes any two
    /// [`ProxyType::None`] proxies the same proxy: only the kind is compared
    /// until there is one.
    fn eq(&self, other: &Self) -> bool {
        if self.kind != other.kind {
            return false;
        }
        if self.kind.is_none() {
            return true;
        }
        self.host == other.host
            && self.ip == other.ip
            && self.port == other.port
            && self.username == other.username
            && self.password == other.password
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_proxy_is_the_default_and_the_only_valid_kind_without_an_address() {
        let proxy = ProxyInfo::none();
        assert_eq!(proxy.kind, ProxyType::None);
        assert!(proxy.is_valid());
        assert!(!proxy.is_address_valid());

        // one that says what it is but not where it is
        let http = ProxyInfo::new(ProxyType::Http, "", "", 0, "", "");
        assert!(!http.is_valid());
        assert!(!http.is_address_valid());
    }

    #[test]
    fn a_proxy_is_valid_once_it_says_where_it_is() {
        let by_host = ProxyInfo::new(ProxyType::Http, "proxy.example", "", 8080, "", "");
        assert!(by_host.is_valid());
        assert!(by_host.is_address_valid());

        let by_ip = ProxyInfo::new(ProxyType::Socks5, "", "10.0.0.1", 1080, "u", "p");
        assert!(by_ip.is_valid());

        // a port of zero is what the C++ calls not valid
        let no_port = ProxyInfo::new(ProxyType::Socks5, "", "10.0.0.1", 0, "", "");
        assert!(!no_port.is_valid());
    }

    #[test]
    fn two_proxies_are_the_same_when_they_say_the_same_thing() {
        let left = ProxyInfo::new(ProxyType::HttpTunnel, "proxy.example", "", 8080, "u", "p");
        let right = ProxyInfo::new(ProxyType::HttpTunnel, "proxy.example", "", 8080, "u", "p");
        assert_eq!(left, right);

        let other_kind = ProxyInfo::new(ProxyType::Http, "proxy.example", "", 8080, "u", "p");
        assert_ne!(left, other_kind);

        let other_password =
            ProxyInfo::new(ProxyType::HttpTunnel, "proxy.example", "", 8080, "u", "q");
        assert_ne!(left, other_password);
    }

    #[test]
    fn any_two_absent_proxies_are_the_same_proxy() {
        // `operator==` compares the kind only while there is no proxy, so what
        // one of them carries in the other fields is not looked at
        let left = ProxyInfo::new(ProxyType::None, "proxy.example", "10.0.0.1", 8080, "u", "p");
        let right = ProxyInfo::none();
        assert_eq!(left, right);
    }
}
