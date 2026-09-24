//! `mars/stn/src/proxy_test.cc` — whether a proxy is one a task can go through.
//!
//! The C++ asks one question of a proxy — `ProxyIsAvailable` — and answers it by
//! going through it: it connects (to the proxy itself when it is an http one,
//! and to the host the test is about *through* it when it is not), writes one
//! `GET`, and reads the status of whatever comes back. [`ProxyTest::test`] is
//! that run, and [`Verdict`] is what the status said, told as a value:
//! [`Verdict::is_available`] is the C++'s own question about it, which is 200,
//! 497, or a redirect.
//!
//! The run is a short link's own, and two things are not the short link's: the
//! request is a `GET` of `/` — of `http://host/` when the proxy is an http one,
//! which is asked for the whole url — and what comes back is judged on its
//! status alone, with no body to answer a task with. The connect is the long
//! link's: `ComplexConnect` with the long link's timeout and interval, which the
//! port hands the operator as [`SocketOperator::set_ip_connection_timeout`].
//!
//! The reads of the answer are an argument, as they are everywhere else in this
//! crate: [`ProxyTest::test`] is the C++'s `while (true)` loop, and what it is
//! handed is what one `Recv` of the socket gave — [`Err`] with the platform's
//! word for why, and [`Ok`] with what came, which is nothing at all when the
//! peer hung up.
//!
//! The `std::map` of headers `__ReadWrite` fills in is not ported: the C++
//! writes its fields straight into the builder and never looks at the map
//! again.

use mars_comm::http::{
    Builder, CsMode, Method, Parser, RequestLine, Version, HOST, PROXY_AUTHORIZATION,
};
use mars_comm::{LocalIpStack, ProxyInfo, ProxyType, SocketAddress};

use crate::config::{LONGLINK_CONN_INTERVAL_MS, LONGLINK_CONN_TIMEOUT_MS};
use crate::short_link::ETIMEDOUT;
use crate::shortlink::authorization;
use crate::socket_operator::{SocketFd, SocketOperator};

/// `TEST_PORT` — the port the host under test is reached on, which is the one
/// an http proxy is *not* reached on: a proxy is reached on its own.
pub const TEST_PORT: u16 = 80;

/// `BUFFER_SIZE` — how much of the answer one read asks the socket for, which
/// is what a host hands its `Recv`: the port's own reads are arguments.
pub const BUFFER_SIZE: usize = 8 * 1024;

/// `5000` — how long a read of the answer waits, which the C++ hands
/// `block_socket_recv` in milliseconds.
pub const READ_TIMEOUT_MS: i32 = 5000;

/// `DnsUtil::GetDNS().GetHostByName(_host)` — the ips of a host, which is how a
/// proxy or a host named by host is reached.
pub type Dns = dyn FnMut(&str) -> Vec<String> + Send;
/// `local_ipstack_detect()` — what the local network carries, which is what an
/// address is mapped onto when there is no proxy to go through.
pub type LocalStack = dyn FnMut() -> LocalIpStack + Send;

/// What the test came back with: the C++'s `status_code`, which is `0` — and
/// therefore not available — whenever the answer never said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// No status at all: the proxy is not one, there is nothing to connect to,
    /// the connect failed, the `GET` did not go out, or what came back was not
    /// an answer.
    Failed,
    /// An answer came back, and this is its status — whether or not it is one
    /// that makes the proxy available.
    Status(i32),
}

impl Verdict {
    /// `status_code == 200 || status_code == 497 || (status_code > 300 &&
    /// status_code < 400)` — the C++'s question, and the only one
    /// `ProxyIsAvailable` asks of what came back.
    ///
    /// 497 is what nginx answers a request that is not https with, and a
    /// redirect is a proxy that answered, which is all the test wants to know.
    /// Neither 300 nor 400 is in between: the C++ compares with `>` and `<`.
    pub fn is_available(&self) -> bool {
        match self {
            Self::Failed => false,
            Self::Status(200) | Self::Status(497) => true,
            Self::Status(status) => (301..=399).contains(status),
        }
    }

    /// The status, which is [`None`] for a test that never got an answer — the
    /// C++'s `0`, which is no status it knows either.
    pub fn status(&self) -> Option<i32> {
        match self {
            Self::Failed => None,
            Self::Status(status) => Some(*status),
        }
    }
}

/// `ProxyTest` — one question about one proxy, answered by going through it.
///
/// The C++'s is a singleton with a breaker of its own; this one is a value a
/// host makes, and the breaker it asks is the operator's
/// ([`SocketOperator::breaker`]), which is the one its blocking calls listen to
/// anyway.
pub struct ProxyTest {
    operator: Option<Box<dyn SocketOperator>>,
    dns: Option<Box<Dns>>,
    local_stack: Option<Box<LocalStack>>,
}

impl ProxyTest {
    /// `ProxyTest()` — a test with nothing to do it with: a host hands it an
    /// operator, and dns and the local stack besides.
    pub fn new() -> Self {
        Self {
            operator: None,
            dns: None,
            local_stack: None,
        }
    }

    /// `socketOperator_` — the sockets the test is made on.
    pub fn set_socket_operator(&mut self, operator: impl SocketOperator + 'static) {
        self.operator = Some(Box::new(operator));
    }

    /// `DnsUtil::GetDNS().GetHostByName(_host)` — how a proxy, or the host under
    /// test, named by host is reached.
    pub fn set_dns(&mut self, dns: impl FnMut(&str) -> Vec<String> + Send + 'static) {
        self.dns = Some(Box::new(dns));
    }

    /// `local_ipstack_detect()` — what an address is mapped onto when there is
    /// no proxy to go through. Unset is [`LocalIpStack::None`], which is a
    /// network that carries nothing to map onto.
    pub fn set_local_stack(&mut self, stack: impl FnMut() -> LocalIpStack + Send + 'static) {
        self.local_stack = Some(Box::new(stack));
    }

    /// `ProxyIsAvailable(_proxy_info, _host, _hardcode_ips)` — whether a task
    /// can go through this proxy, which is [`ProxyTest::test`]'s answer put to
    /// the C++'s own question.
    pub fn is_available(
        &mut self,
        proxy: &ProxyInfo,
        host: &str,
        hardcode_ips: &[String],
        reads: impl Iterator<Item = Result<Vec<u8>, i32>>,
    ) -> bool {
        self.test(proxy, host, hardcode_ips, reads).is_available()
    }

    /// The whole run of `__Connect` and `__ReadWrite`: the connect, one `GET`,
    /// and the reads until the answer is whole — or until the socket says it
    /// will not be.
    ///
    /// The socket is closed whatever became of it, which is what the C++ does
    /// after `__ReadWrite` came back.
    pub fn test(
        &mut self,
        proxy: &ProxyInfo,
        host: &str,
        hardcode_ips: &[String],
        reads: impl Iterator<Item = Result<Vec<u8>, i32>>,
    ) -> Verdict {
        // `!_proxy_info.IsValid() || (_host.empty() && _hardcode_ips.empty())`
        if !proxy.is_valid() || (host.is_empty() && hardcode_ips.is_empty()) {
            return Verdict::Failed;
        }

        let Some(socket) = self.connect(proxy, host, hardcode_ips) else {
            return Verdict::Failed;
        };

        let request = self.request(proxy, host);
        let verdict = match self.write(socket, &request) {
            Ok(_) if !self.is_broken() => self.read(reads),
            // a `GET` that did not go out, or one the app broke off
            _ => Verdict::Failed,
        };
        self.close(socket);
        verdict
    }

    /// `__Connect` — the socket the test is made on, or [`None`], which is the
    /// C++'s `INVALID_SOCKET`: a proxy named by host that dns could not name, a
    /// host that has neither a dns answer nor a hardcode ip, or a connect that
    /// failed.
    fn connect(
        &mut self,
        proxy: &ProxyInfo,
        host: &str,
        hardcode_ips: &[String],
    ) -> Option<SocketFd> {
        // what the proxy is reached at: its own ip, or the first one dns named
        // for the host it was given as. A host dns could not name is no socket
        // at all, which is what the C++ answers for it
        let proxy_ip = if proxy.kind.is_none() {
            None
        } else if !proxy.ip.is_empty() {
            Some(proxy.ip.clone())
        } else {
            Some(self.dns(&proxy.host).into_iter().next()?)
        };

        let stack = self.local_stack();
        let addresses = match proxy.kind {
            // an http proxy is connected *to*, so it is the only address there
            // is: the host under test is what the request asks it for
            ProxyType::Http => {
                let mut address =
                    SocketAddress::new(&proxy_ip.clone().unwrap_or_default(), proxy.port);
                address.v4_to_v6_address(stack);
                vec![address]
            }
            _ => {
                let mut test_ips = self.dns(host);
                if test_ips.is_empty() {
                    test_ips = hardcode_ips.to_vec();
                }
                if test_ips.is_empty() {
                    return None;
                }
                test_ips
                    .iter()
                    .map(|ip| {
                        let mut address = SocketAddress::new(ip, TEST_PORT);
                        // a proxy is reached at the address it is; only a
                        // connect with no proxy in it is mapped onto the stack
                        // the local network carries
                        if proxy.kind.is_none() {
                            address.v4_to_v6_address(stack);
                        }
                        address
                    })
                    .collect()
            }
        };
        if addresses.is_empty() {
            return None;
        }

        // the proxy the operator is given is the one dns named, not the host the
        // app told it about; a tunnel or a socks5 one is connected *through*,
        // which the operator decides from the proxy itself
        let connect_proxy = match &proxy_ip {
            Some(ip) => ProxyInfo {
                ip: ip.clone(),
                port: proxy.port,
                ..proxy.clone()
            },
            None => ProxyInfo::none(),
        };

        if let Some(operator) = self.operator.as_mut() {
            // `ComplexConnect(kLonglinkConnTimeout, kLonglinkConnInteral, …)`
            operator.set_ip_connection_timeout(LONGLINK_CONN_TIMEOUT_MS, LONGLINK_CONN_INTERVAL_MS);
        }
        let socket = self.open(&addresses, &connect_proxy);
        socket.is_valid().then_some(socket)
    }

    /// `__ReadWrite` — the request: a `GET` of the host under test, which an
    /// http proxy is asked for as a whole url and anything else as `/`.
    ///
    /// The head is the C++'s own four fields, the `Host` of the test, and a
    /// `Proxy-Authorization` when there is an account to log into an http proxy
    /// with — the same field [`crate::shortlink`] writes for a task.
    fn request(&self, proxy: &ProxyInfo, host: &str) -> Vec<u8> {
        let url = if proxy.kind == ProxyType::Http {
            // the `/` is the C++'s own: without it some proxy servers answer
            // 400 bad request
            format!("http://{host}/")
        } else {
            "/".to_string()
        };

        let mut builder = Builder::new(CsMode::Request);
        *builder.request_mut() = RequestLine::new(Method::Get, &url, Version::V1_1);

        let fields = builder.fields_mut();
        fields.set_accept_all();
        fields.set_user_agent_micro_message();
        fields.set_cache_control_no_cache();
        fields.set_connection_close();
        fields.set(HOST, host);
        if let Some(authorization) = authorization(proxy) {
            fields.set(PROXY_AUTHORIZATION, &authorization);
        }

        builder.header_to_buffer().unwrap_or_default()
    }

    /// The read loop of `__ReadWrite` — one read at a time until the answer is
    /// whole, or until the socket says it will not be.
    ///
    /// What the loop ends with is the status of the answer as far as it came in:
    /// a first line is enough to say one, which is why a socket that hung up
    /// after it still leaves the test with the status it read.
    ///
    /// The socket the C++ hands `block_socket_recv` is not one here: the reads
    /// are an argument, so there is nothing to read from.
    fn read(&mut self, reads: impl Iterator<Item = Result<Vec<u8>, i32>>) -> Verdict {
        let mut answer = Parser::new();
        let mut status = None;

        for read in reads {
            // `testproxybreak_.IsBreak()` — the app gave up, which ends the
            // loop before anything the read said
            if self.is_broken() {
                break;
            }
            match read {
                // `recv_ret < 0` — a read that went wrong
                Err(err_code) if err_code != ETIMEDOUT => break,
                // `recv_ret == 0 && SOCKET_ERRNO(ETIMEDOUT)` — a read that
                // timed out, which is read again: the C++ has no limit on how
                // often
                Err(_) => continue,
                // `recv_ret == 0` — the peer hung up
                Ok(bytes) if bytes.is_empty() => break,
                Ok(bytes) => {
                    let recv_status = answer.recv(&bytes);
                    if answer.is_first_line_ready() {
                        status = Some(answer.status().status_code);
                    }
                    if recv_status.is_error() || recv_status.is_end() {
                        break;
                    }
                }
            }
        }

        match status {
            Some(status) => Verdict::Status(status),
            None => Verdict::Failed,
        }
    }

    fn open(&mut self, addresses: &[SocketAddress], proxy: &ProxyInfo) -> SocketFd {
        match self.operator.as_mut() {
            Some(operator) => operator.connect(addresses, proxy),
            None => SocketFd::INVALID,
        }
    }

    /// `block_socket_send` — the `GET` goes out, whole and once: the C++ hands
    /// no timeout of its own, which is the socket's to pick.
    fn write(&mut self, socket: SocketFd, request: &[u8]) -> Result<usize, i32> {
        match self.operator.as_mut() {
            Some(operator) => operator.send(socket, request, -1),
            None => Err(0),
        }
    }

    /// `socket_close(sock)` — whatever became of the test, the socket is not
    /// one anything else uses.
    fn close(&mut self, socket: SocketFd) {
        if let Some(operator) = self.operator.as_mut() {
            operator.close(socket);
        }
    }

    /// Whether the app broke the test off: `socketOperator_->Breaker()`.
    fn is_broken(&mut self) -> bool {
        match self.operator.as_mut() {
            Some(operator) => operator.breaker().is_break(),
            None => false,
        }
    }

    fn dns(&mut self, host: &str) -> Vec<String> {
        match self.dns.as_mut() {
            Some(dns) => dns(host),
            None => Vec::new(),
        }
    }

    fn local_stack(&mut self) -> LocalIpStack {
        match self.local_stack.as_mut() {
            Some(stack) => stack(),
            None => LocalIpStack::None,
        }
    }
}

impl Default for ProxyTest {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ProxyTest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyTest")
            .field("operator", &self.operator.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::socket_operator::{OpBreaker, SocketProfile};
    use crate::Task;

    /// What the host was asked for, and what it answers with.
    #[derive(Clone, Default)]
    struct Seen {
        addresses: Arc<Mutex<Vec<Vec<SocketAddress>>>>,
        proxies: Arc<Mutex<Vec<ProxyInfo>>>,
        timeouts: Arc<Mutex<Vec<(u32, u32)>>>,
        sent: Arc<Mutex<Vec<Vec<u8>>>>,
        closed: Arc<Mutex<Vec<SocketFd>>>,
        /// A connect that does not happen, and a write that does not: `None` is
        /// one that does.
        fail_connect: Arc<Mutex<bool>>,
        fail_send: Arc<Mutex<Option<i32>>>,
        broken: Arc<Mutex<bool>>,
    }

    impl Seen {
        fn sent(&self) -> Vec<Vec<u8>> {
            self.sent.lock().unwrap().clone()
        }

        fn closed(&self) -> Vec<SocketFd> {
            self.closed.lock().unwrap().clone()
        }

        fn only_request(&self) -> String {
            String::from_utf8_lossy(&self.sent()[0]).to_string()
        }
    }

    /// A pipe the test wakes itself.
    #[derive(Default)]
    struct Breaker(Arc<Mutex<bool>>);

    impl OpBreaker for Breaker {
        fn is_break(&mut self) -> bool {
            *self.0.lock().unwrap()
        }

        fn break_(&mut self) -> bool {
            *self.0.lock().unwrap() = true;
            true
        }
    }

    /// The host's sockets: the numbers it hands out, and what it was asked to do
    /// with them.
    struct Host {
        seen: Seen,
        next: i64,
        breaker: Breaker,
    }

    impl Host {
        fn new(seen: Seen) -> Self {
            Self {
                breaker: Breaker(seen.broken.clone()),
                seen,
                next: 3,
            }
        }
    }

    impl SocketOperator for Host {
        fn connect(&mut self, addresses: &[SocketAddress], proxy: &ProxyInfo) -> SocketFd {
            self.seen.addresses.lock().unwrap().push(addresses.to_vec());
            self.seen.proxies.lock().unwrap().push(proxy.clone());
            if *self.seen.fail_connect.lock().unwrap() {
                return SocketFd::INVALID;
            }
            let socket = SocketFd(self.next);
            self.next += 1;
            socket
        }

        fn send(
            &mut self,
            _socket: SocketFd,
            buffer: &[u8],
            _timeout_ms: i32,
        ) -> Result<usize, i32> {
            self.seen.sent.lock().unwrap().push(buffer.to_vec());
            match *self.seen.fail_send.lock().unwrap() {
                Some(err_code) => Err(err_code),
                None => Ok(buffer.len()),
            }
        }

        fn recv(
            &mut self,
            _socket: SocketFd,
            _max_size: usize,
            _timeout_ms: i32,
            _wait_full_size: bool,
        ) -> Result<Vec<u8>, i32> {
            Ok(Vec::new())
        }

        fn close(&mut self, socket: SocketFd) {
            self.seen.closed.lock().unwrap().push(socket);
        }

        fn identify(&self, socket: SocketFd) -> String {
            format!("{}@TCP", socket.0)
        }

        fn protocol(&self) -> i32 {
            Task::TRANSPORT_PROTOCOL_TCP
        }

        fn error_desc(&self, error_code: i32) -> String {
            format!("error {error_code}")
        }

        fn profile(&self) -> SocketProfile {
            SocketProfile::default()
        }

        fn breaker(&mut self) -> &mut dyn OpBreaker {
            &mut self.breaker
        }

        fn create_stream(&mut self, _socket: SocketFd) -> SocketFd {
            SocketFd::INVALID
        }

        fn set_ip_connection_timeout(&mut self, v4_timeout_ms: u32, v6_timeout_ms: u32) {
            self.seen
                .timeouts
                .lock()
                .unwrap()
                .push((v4_timeout_ms, v6_timeout_ms));
        }
    }

    /// The dns of the whole test: it knows the host under test, and the host of
    /// the proxy.
    fn dns(host: &str) -> Vec<String> {
        match host {
            "short.weixin.qq.com" => vec!["183.3.226.35".to_string()],
            "proxy.example" => vec!["10.0.0.1".to_string()],
            _ => Vec::new(),
        }
    }

    /// A test on the host's sockets, wired to the dns above and to a log of what
    /// the host was asked for.
    fn test_of(seen: &Seen) -> ProxyTest {
        let mut test = ProxyTest::new();
        test.set_socket_operator(Host::new(seen.clone()));
        test.set_dns(dns);
        test
    }

    fn test() -> (ProxyTest, Seen) {
        let seen = Seen::default();
        let link = test_of(&seen);
        (link, seen)
    }

    /// An http proxy on its own ip.
    fn http_proxy() -> ProxyInfo {
        ProxyInfo::new(ProxyType::Http, "", "10.0.0.1", 8080, "", "")
    }

    /// The reads of one answer, which is the whole of it.
    fn answer(status: i32) -> Vec<Result<Vec<u8>, i32>> {
        vec![Ok(format!(
            "HTTP/1.1 {status} OK\r\nContent-Length: 0\r\n\r\n"
        )
        .into_bytes())]
    }

    fn reads_of(chunks: &[&str]) -> Vec<Result<Vec<u8>, i32>> {
        chunks
            .iter()
            .map(|chunk| Ok(chunk.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn a_proxy_that_is_not_one_is_not_available() {
        let (mut test, seen) = test();
        // an http proxy that does not say where it is
        let verdict = test.test(
            &ProxyInfo::new(ProxyType::Http, "", "", 0, "", ""),
            "short.weixin.qq.com",
            &[],
            answer(200).into_iter(),
        );
        assert_eq!(verdict, Verdict::Failed);
        assert!(!verdict.is_available());
        assert_eq!(verdict.status(), None);
        assert!(
            seen.addresses.lock().unwrap().is_empty(),
            "no connect at all"
        );
    }

    #[test]
    fn a_host_with_no_hardcode_ips_to_fall_back_on_is_not_available() {
        let (mut test, seen) = test();
        let verdict = test.test(&http_proxy(), "", &[], answer(200).into_iter());
        assert_eq!(verdict, Verdict::Failed);
        assert!(seen.addresses.lock().unwrap().is_empty());

        // a hardcode ip is enough, even with no host to resolve
        let hardcode = vec!["183.3.226.35".to_string()];
        let verdict = test.test(&http_proxy(), "", &hardcode, answer(200).into_iter());
        assert_eq!(verdict, Verdict::Status(200));
        assert!(verdict.is_available());
    }

    #[test]
    fn an_http_proxy_is_one_the_test_connects_to() {
        let (mut test, seen) = test();
        test.test(
            &http_proxy(),
            "short.weixin.qq.com",
            &[],
            answer(200).into_iter(),
        );

        let addresses = seen.addresses.lock().unwrap().clone();
        assert_eq!(addresses.len(), 1, "one connect, on the proxy itself");
        assert_eq!(addresses[0].len(), 1, "and one address in it");
        let address = &addresses[0][0];
        assert_eq!(address.ip(), "10.0.0.1");
        assert_eq!(address.port(), 8080);
        // and the proxy is the one the operator is told to go through
        let proxies = seen.proxies.lock().unwrap().clone();
        assert_eq!(proxies[0], http_proxy());
    }

    #[test]
    fn a_proxy_named_by_host_is_the_one_dns_named() {
        let (mut test, seen) = test();
        let by_host = ProxyInfo::new(ProxyType::Http, "proxy.example", "", 8080, "", "");
        test.test(
            &by_host,
            "short.weixin.qq.com",
            &[],
            answer(200).into_iter(),
        );

        let addresses = seen.addresses.lock().unwrap().clone();
        assert_eq!(addresses[0][0].ip(), "10.0.0.1", "what dns answered");
        let proxies = seen.proxies.lock().unwrap().clone();
        assert_eq!(
            proxies[0],
            ProxyInfo {
                ip: "10.0.0.1".to_string(),
                ..by_host
            },
            "the operator is given the ip, not the host"
        );
    }

    #[test]
    fn a_proxy_host_dns_could_not_name_is_no_socket() {
        let (mut test, seen) = test();
        let unknown = ProxyInfo::new(ProxyType::Http, "unknown.example", "", 8080, "", "");
        let verdict = test.test(
            &unknown,
            "short.weixin.qq.com",
            &[],
            answer(200).into_iter(),
        );
        assert_eq!(verdict, Verdict::Failed);
        assert!(seen.addresses.lock().unwrap().is_empty());
        assert!(seen.sent().is_empty(), "nothing went out either");
    }

    #[test]
    fn a_tunnel_proxy_is_one_the_test_connects_through() {
        let (mut test, seen) = test();
        let tunnel = ProxyInfo::new(ProxyType::Socks5, "", "10.0.0.1", 1080, "mars", "secret");
        test.test(&tunnel, "short.weixin.qq.com", &[], answer(200).into_iter());

        let addresses = seen.addresses.lock().unwrap().clone();
        assert_eq!(addresses.len(), 1);
        // the host under test, on the test port — and not mapped onto the local
        // stack, which only a connect with no proxy in it is
        assert_eq!(addresses[0][0].ip(), "183.3.226.35");
        assert_eq!(addresses[0][0].port(), TEST_PORT);
        assert!(
            !addresses[0][0].is_v6(),
            "a proxy is reached at the address it is"
        );
        let proxies = seen.proxies.lock().unwrap().clone();
        assert_eq!(proxies[0], tunnel, "and the proxy goes with it");
    }

    #[test]
    fn no_proxy_is_a_connect_to_the_host_on_the_test_port() {
        let (mut test, seen) = test();
        test.test(
            &ProxyInfo::none(),
            "short.weixin.qq.com",
            &[],
            answer(200).into_iter(),
        );

        let addresses = seen.addresses.lock().unwrap().clone();
        assert_eq!(addresses[0][0].ip(), "183.3.226.35");
        assert_eq!(addresses[0][0].port(), TEST_PORT);
        let proxies = seen.proxies.lock().unwrap().clone();
        assert_eq!(proxies[0], ProxyInfo::none(), "no proxy to go through");
    }

    #[test]
    fn a_host_dns_could_not_name_is_the_hardcode_ips() {
        let (mut test, seen) = test();
        let hardcode = vec!["183.3.226.36".to_string(), "183.3.226.37".to_string()];
        test.test(
            &ProxyInfo::none(),
            "unknown.example",
            &hardcode,
            answer(200).into_iter(),
        );

        let addresses = seen.addresses.lock().unwrap().clone();
        assert_eq!(addresses[0].len(), 2, "both of them, in the order given");
        assert_eq!(addresses[0][0].ip(), "183.3.226.36");
        assert_eq!(addresses[0][1].ip(), "183.3.226.37");
    }

    #[test]
    fn a_host_dns_could_not_name_and_no_hardcode_ips_is_no_socket() {
        let (mut test, seen) = test();
        let verdict = test.test(
            &ProxyInfo::none(),
            "unknown.example",
            &[],
            answer(200).into_iter(),
        );
        assert_eq!(verdict, Verdict::Failed);
        assert!(seen.addresses.lock().unwrap().is_empty());
    }

    #[test]
    fn the_addresses_with_no_proxy_are_mapped_onto_the_local_stack() {
        let (mut test, seen) = test();
        test.set_local_stack(|| LocalIpStack::IPv6);
        test.test(
            &ProxyInfo::none(),
            "short.weixin.qq.com",
            &[],
            answer(200).into_iter(),
        );

        let addresses = seen.addresses.lock().unwrap().clone();
        assert!(
            addresses[0][0].is_v6(),
            "a v4 address mapped onto a v6 network"
        );
    }

    #[test]
    fn an_answer_of_two_hundred_is_available() {
        let (mut test, _seen) = test();
        let verdict = test.test(
            &http_proxy(),
            "short.weixin.qq.com",
            &[],
            answer(200).into_iter(),
        );
        assert_eq!(verdict, Verdict::Status(200));
        assert!(verdict.is_available());
        assert_eq!(verdict.status(), Some(200));
    }

    #[test]
    fn the_statuses_the_cpp_calls_available() {
        let available = [200, 497, 301, 302, 307, 399];
        for status in available {
            assert!(
                Verdict::Status(status).is_available(),
                "{status} is one the C++ calls available"
            );
        }
        for status in [100, 204, 300, 400, 404, 500] {
            assert!(!Verdict::Status(status).is_available(), "{status} is not");
        }
        assert!(!Verdict::Failed.is_available());
    }

    #[test]
    fn an_answer_that_came_in_two_reads_is_whole_on_the_second() {
        let (mut test, _seen) = test();
        let reads = reads_of(&["HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhel", "lo"]);
        let verdict = test.test(&http_proxy(), "short.weixin.qq.com", &[], reads.into_iter());
        assert_eq!(verdict, Verdict::Status(200));
    }

    #[test]
    fn a_read_that_timed_out_is_read_again() {
        let (mut test, _seen) = test();
        let mut reads = vec![Err(ETIMEDOUT), Err(ETIMEDOUT)];
        reads.extend(answer(200));
        let verdict = test.test(&http_proxy(), "short.weixin.qq.com", &[], reads.into_iter());
        assert_eq!(verdict, Verdict::Status(200));
    }

    #[test]
    fn a_read_that_failed_is_no_answer() {
        let (mut test, _seen) = test();
        // a platform's own word for it, which is not a timeout
        let reads = vec![Err(104)];
        let verdict = test.test(&http_proxy(), "short.weixin.qq.com", &[], reads.into_iter());
        assert_eq!(verdict, Verdict::Failed);
    }

    #[test]
    fn a_socket_that_hung_up_is_no_answer() {
        let (mut test, _seen) = test();
        let verdict = test.test(
            &http_proxy(),
            "short.weixin.qq.com",
            &[],
            vec![Ok(Vec::new())].into_iter(),
        );
        assert_eq!(verdict, Verdict::Failed);
    }

    #[test]
    fn a_first_line_and_then_a_socket_that_hung_up_is_the_status_it_read() {
        let (mut test, _seen) = test();
        // the C++ takes the status as soon as the first line is whole, and
        // keeps it when the read that follows brings nothing
        let reads = reads_of(&["HTTP/1.1 302 Found\r\n", ""]);
        let verdict = test.test(&http_proxy(), "short.weixin.qq.com", &[], reads.into_iter());
        assert_eq!(verdict, Verdict::Status(302));
        assert!(verdict.is_available());
    }

    #[test]
    fn an_answer_that_is_not_one_is_no_answer() {
        let (mut test, _seen) = test();
        let reads = reads_of(&["not an answer at all\r\n\r\n"]);
        let verdict = test.test(&http_proxy(), "short.weixin.qq.com", &[], reads.into_iter());
        assert_eq!(verdict, Verdict::Failed, "not a status line, so no status");
    }

    #[test]
    fn a_socket_that_hung_up_before_the_head_ended_is_the_status_it_read() {
        let (mut test, _seen) = test();
        // a first line that came whole, and a head that never did: the C++ has
        // the status by then, and stops reading
        let reads = reads_of(&["HTTP/1.1 200 OK\r\nContent-Length: 5", ""]);
        let verdict = test.test(&http_proxy(), "short.weixin.qq.com", &[], reads.into_iter());
        assert_eq!(verdict, Verdict::Status(200));
        assert!(verdict.is_available());
    }

    #[test]
    fn a_write_that_did_not_go_out_is_not_available() {
        let (mut test, seen) = test();
        *seen.fail_send.lock().unwrap() = Some(9);
        let verdict = test.test(
            &http_proxy(),
            "short.weixin.qq.com",
            &[],
            answer(200).into_iter(),
        );
        assert_eq!(verdict, Verdict::Failed);
        assert!(
            seen.closed.lock().unwrap().len() == 1,
            "the socket is closed whatever became of the write"
        );
    }

    #[test]
    fn a_connect_that_failed_is_not_available() {
        let (mut test, seen) = test();
        *seen.fail_connect.lock().unwrap() = true;
        let verdict = test.test(
            &http_proxy(),
            "short.weixin.qq.com",
            &[],
            answer(200).into_iter(),
        );
        assert_eq!(verdict, Verdict::Failed);
        assert!(seen.sent().is_empty(), "nothing goes out on no socket");
        assert!(seen.closed.lock().unwrap().is_empty());
    }

    #[test]
    fn a_test_the_app_broke_off_is_not_available() {
        let (mut test, seen) = test();
        *seen.broken.lock().unwrap() = true;
        let verdict = test.test(
            &http_proxy(),
            "short.weixin.qq.com",
            &[],
            answer(200).into_iter(),
        );
        assert_eq!(verdict, Verdict::Failed);
        assert!(
            seen.closed.lock().unwrap().len() == 1,
            "and the socket is closed"
        );
    }

    #[test]
    fn the_socket_is_closed_when_the_test_is_over() {
        let (mut test, seen) = test();
        test.test(
            &http_proxy(),
            "short.weixin.qq.com",
            &[],
            answer(200).into_iter(),
        );
        assert_eq!(seen.closed(), vec![SocketFd(3)]);
    }

    #[test]
    fn the_request_is_a_get_of_the_whole_url_through_an_http_proxy() {
        let (mut test, seen) = test();
        test.test(
            &http_proxy(),
            "short.weixin.qq.com",
            &[],
            answer(200).into_iter(),
        );
        assert_eq!(
            seen.only_request(),
            "GET http://short.weixin.qq.com/ HTTP/1.1\r\n\
             Accept: */*\r\n\
             User-Agent: MicroMessenger Client\r\n\
             Cache-Control: no-cache\r\n\
             Connection: close\r\n\
             Host: short.weixin.qq.com\r\n\
             \r\n"
        );
    }

    #[test]
    fn the_request_is_a_get_of_the_root_when_there_is_no_proxy() {
        let (mut test, seen) = test();
        test.test(
            &ProxyInfo::none(),
            "short.weixin.qq.com",
            &[],
            answer(200).into_iter(),
        );
        let request = seen.only_request();
        assert!(
            request.starts_with("GET / HTTP/1.1\r\n"),
            "the path and nothing else: {request}"
        );
        assert!(request.contains("Host: short.weixin.qq.com\r\n"));
    }

    #[test]
    fn an_http_proxy_with_an_account_is_logged_in_with() {
        let (mut test, seen) = test();
        let proxy = ProxyInfo::new(ProxyType::Http, "", "10.0.0.1", 8080, "mars", "secret");
        test.test(&proxy, "short.weixin.qq.com", &[], answer(200).into_iter());
        assert!(
            seen.only_request()
                .contains("Proxy-Authorization: Basic bWFyczpzZWNyZXQ=\r\n"),
            "the account in base64"
        );
    }

    #[test]
    fn a_proxy_that_is_not_an_http_one_is_not_logged_in_with() {
        // even with an account to log in with
        let (mut test, seen) = test();
        let proxy = ProxyInfo::new(ProxyType::Socks5, "", "10.0.0.1", 1080, "mars", "secret");
        test.test(&proxy, "short.weixin.qq.com", &[], answer(200).into_iter());
        assert!(!seen.only_request().contains("Proxy-Authorization"));
    }

    #[test]
    fn the_connect_is_the_long_link_s() {
        let (mut test, seen) = test();
        test.test(
            &http_proxy(),
            "short.weixin.qq.com",
            &[],
            answer(200).into_iter(),
        );
        assert_eq!(
            seen.timeouts.lock().unwrap().clone(),
            vec![(LONGLINK_CONN_TIMEOUT_MS, LONGLINK_CONN_INTERVAL_MS)]
        );
    }

    #[test]
    fn a_test_with_no_operator_is_not_one_that_ran() {
        let mut test = ProxyTest::new();
        test.set_dns(dns);
        let verdict = test.test(
            &http_proxy(),
            "short.weixin.qq.com",
            &[],
            answer(200).into_iter(),
        );
        assert_eq!(verdict, Verdict::Failed, "no sockets to do it on");
    }

    #[test]
    fn a_test_that_ran_keeps_asking_the_same_question() {
        let (mut test, seen) = test();
        // the same test twice: the second one is its own connect, its own `GET`
        // and its own socket
        let reads = answer(200);
        test.test(
            &http_proxy(),
            "short.weixin.qq.com",
            &[],
            reads.clone().into_iter(),
        );
        assert!(test.is_available(&http_proxy(), "short.weixin.qq.com", &[], reads.into_iter()));
        assert_eq!(seen.sent().len(), 2);
        assert_eq!(seen.closed(), vec![SocketFd(3), SocketFd(4)]);
    }

    #[test]
    fn the_default_test_is_one_with_nothing_to_do_it_with() {
        let test = ProxyTest::default();
        assert!(format!("{test:?}").contains("ProxyTest"));
    }
}
