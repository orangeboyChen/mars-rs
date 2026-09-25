//! `mars/sdt/src/checkimpl/` — what the four probes are asked, and what they
//! answer back.

use mars_sdt::checkimpl::{Answer, Ask, PingStatus, Query};

#[test]
fn an_answer_read_as_another_probe_is_that_probe_failing() {
    let dns = Answer::Dns {
        error_code: 0,
        rtt: 12,
        ips: vec!["1.2.3.4".to_owned()],
    };
    assert_eq!(dns.dns().0, 0);
    assert_eq!(dns.dns().1, 12);
    assert_eq!(dns.dns().2, ["1.2.3.4".to_owned()]);
    // the same answer, read as the three probes it is not
    assert_eq!(dns.tcp(), (-1, false, 0));
    assert_eq!(dns.http(), (-1, 0, 0));
    assert_eq!(dns.ping(), (-1, 0, None));

    let tcp = Answer::Tcp {
        error_code: 0,
        is_noop_resp: true,
        rtt: 5,
    };
    assert_eq!(tcp.tcp(), (0, true, 5));
    assert!(tcp.dns().2.is_empty());
    assert_eq!(tcp.http(), (-1, 0, 0));
    assert_eq!(tcp.ping(), (-1, 0, None));

    let http = Answer::Http {
        error_code: 0,
        status_code: 200,
        rtt: 40,
    };
    assert_eq!(http.http(), (0, 200, 40));
    assert!(http.dns().2.is_empty());
    assert_eq!(http.tcp(), (-1, false, 0));
    assert_eq!(http.ping(), (-1, 0, None));

    let status = PingStatus::new(1.0, 0.0);
    let ping = Answer::Ping {
        error_code: 0,
        rtt: 60,
        status: Some(status),
    };
    assert_eq!(ping.ping(), (0, 60, Some(&status)));
    assert!(ping.dns().2.is_empty());
    assert_eq!(ping.tcp(), (-1, false, 0));
    assert_eq!(ping.http(), (-1, 0, 0));
}

#[test]
fn an_answer_nobody_gave_is_every_probe_failing() {
    assert_eq!(Answer::default(), Answer::Nothing);

    let nothing = Answer::Nothing;
    assert!(nothing.dns().2.is_empty());
    assert_eq!(nothing.dns(), (-1, 0, [].as_slice()));
    assert_eq!(nothing.tcp(), (-1, false, 0));
    assert_eq!(nothing.http(), (-1, 0, 0));
    assert_eq!(nothing.ping(), (-1, 0, None));
}

#[test]
fn a_ping_status_is_what_the_host_made_of_its_pings() {
    let status = PingStatus::default();
    assert_eq!(status.loss_rate, 0.0);
    assert_eq!(status.avgrtt, 0.0);

    // `1.0` is every ping lost, which the C++ calls a failed ping
    let all_lost = PingStatus::new(1.0, 25.5);
    assert_eq!(all_lost.loss_rate, 1.0);
    assert_eq!(all_lost.avgrtt, 25.5);
}

#[test]
fn a_seam_nobody_filled_in_answers_nothing() {
    let ask = Ask::default();
    // the answerer is not something a caller can look at, so all there is to
    // show is that one was made — and that it prints as one
    assert!(format!("{ask:?}").starts_with("Ask"));
}

#[test]
fn the_query_of_each_probe_is_what_the_check_asked_for() {
    // the four questions, as [`crate::activecheck`] asks them
    let dns = Query::Dns {
        domain: "long.host".to_owned(),
        timeout_ms: 3000,
    };
    let tcp = Query::Tcp {
        ip: "1.2.3.4".to_owned(),
        port: 80,
        timeout_ms: 5000,
    };
    let http = Query::Http {
        url: "http://short.host/netcheck".to_owned(),
        timeout_ms: 1000,
    };
    let ping = Query::Ping {
        host: "www.qq.com".to_owned(),
        timeout_s: 0,
    };

    // every one of them is a value a host can keep and compare
    assert_ne!(format!("{dns:?}"), format!("{tcp:?}"));
    assert_eq!(
        ping,
        Query::Ping {
            host: "www.qq.com".to_owned(),
            timeout_s: 0
        }
    );
    assert_ne!(http, dns);
}
