//! `mars/sdt/src/activecheck/` — the four checks, against a host that answers
//! the probes.

use std::sync::{Arc, Mutex};

use marsrs_sdt::activecheck::Check;
use marsrs_sdt::checkimpl::{Answer, Ask, PingStatus, Query};
use marsrs_sdt::netchecker_profile::{CheckRequestProfile, CheckResultProfile};
use marsrs_sdt::report::report_json;
use marsrs_sdt::sdt::{Callback, CheckIPPort, CheckIPPorts, CheckStatus, NetCheckType, TcpErrCode};
use marsrs_sdt::sdt_core::{CancelHandle, SdtCore};
use marsrs_sdt::sdt_logic::SdtLogic;
use marsrs_sdt::{
    DEFAULT_DNS_TIMEOUT, DEFAULT_HTTP_HOST, DEFAULT_PING_COUNT, DEFAULT_PING_HOST,
    HTTP_DEFAULT_TIMEOUT, NET_CHECK_BASIC, NET_CHECK_SHORT, UNUSE_TIMEOUT,
};

/// The hosts of one link: the name they are filed under, the ip, and the port.
fn link(items: &[(&str, &str, u16)]) -> CheckIPPorts {
    items
        .iter()
        .map(|(host, ip, port)| ((*host).to_owned(), vec![CheckIPPort::new(*ip, *port)]))
        .collect()
}

/// One request, as `SdtCore::__InitCheckReq` leaves it.
fn request_of(
    longlink: CheckIPPorts,
    shortlink: CheckIPPorts,
    timeout: u32,
) -> CheckRequestProfile {
    let mut request = CheckRequestProfile::new();
    request.longlink_items = longlink;
    request.shortlink_items = shortlink;
    request.total_timeout = timeout;
    request
}

fn check_of(request: &CheckRequestProfile) -> Check {
    Check::new(CancelHandle::new(), request.total_timeout)
}

/// A host that answers `answer`, and keeps what it was asked.
fn stub(answer: fn(&Query) -> Answer) -> (Ask, Arc<Mutex<Vec<Query>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&asked);
    let ask = Ask::new(move |query: Query| {
        sink.lock().unwrap().push(query.clone());
        answer(&query)
    });
    (ask, asked)
}

/// A host every probe succeeds against, and every probe takes 100 ms of.
fn slow(query: &Query) -> Answer {
    match query {
        Query::Dns { .. } => Answer::Dns {
            error_code: 0,
            rtt: 100,
            local_dns: "8.8.8.8".to_owned(),
            ips: vec!["1.1.1.1".to_owned(), "2.2.2.2".to_owned()],
        },
        Query::Tcp { .. } => Answer::Tcp {
            sent: 0,
            received: 0,
            is_noop_resp: true,
            conntime: 40,
            rtt: 100,
        },
        Query::Http { .. } => Answer::Http {
            error_code: 0,
            status_code: 200,
            rtt: 100,
        },
        Query::Ping { .. } => Answer::Ping {
            error_code: 0,
            rtt: 100,
            status: Some(PingStatus::new(0.0, 12.5)),
        },
    }
}

#[test]
fn a_host_that_cannot_probe_fails_every_check_it_runs() {
    let mut request = request_of(
        link(&[("long.host", "1.2.3.4", 80)]),
        link(&[("short.host", "5.6.7.8", 443)]),
        UNUSE_TIMEOUT,
    );
    let mut ask = Ask::default();
    let mut check = check_of(&request);

    for kind in [
        NetCheckType::PingCheck,
        NetCheckType::DnsCheck,
        NetCheckType::TcpCheck,
        NetCheckType::HttpCheck,
    ] {
        assert!(check.start_do_check(kind, &mut request, &mut ask, 2, "/netcheck"));
    }

    let results = &request.checkresult_profiles;
    let kinds: Vec<NetCheckType> = results
        .iter()
        .filter_map(CheckResultProfile::kind)
        .collect();
    // No ping profile at all: a host with no ICMP to send is a platform the
    // C++ skips the ping on, so the check did not run and failed nothing.
    assert_eq!(
        kinds,
        vec![
            NetCheckType::DnsCheck,
            NetCheckType::DnsCheck,
            NetCheckType::TcpCheck,
            NetCheckType::HttpCheck,
        ]
    );

    // a host nobody resolved
    assert_eq!(results[0].domain_name, "long.host");
    assert_eq!(results[0].error_code, -1);
    assert!(results[0].ip1.is_empty());
    assert!(results[0].ip2.is_empty());
    assert_eq!(results[1].domain_name, "short.host");

    // a noop that did not go out: not the socket's own error, but `kSndRcvErr`
    assert_eq!(results[2].error_code, TcpErrCode::SndRcvErr.as_i32());
    assert_eq!(results[2].rtt, 0);
    assert_eq!(results[2].port, 80);

    // a request nobody sent, to the CGI the core was given
    assert_eq!(results[3].url, "http://short.host/netcheck");
    assert_eq!(results[3].status_code, 0);
    // `error_code` is what the app reads for "how the probe went", and a
    // profile that was never given one says `0`, which is "worked": a request
    // nobody sent has to say so here, and not only in the status the run
    // reports
    assert_eq!(results[3].error_code, -1);

    assert_eq!(request.check_status, CheckStatus::CheckFinish);
    // a run without a timeout is never a run that ran out of one
    assert_eq!(check.remaining(), UNUSE_TIMEOUT);
}

/// A host with a resolver, a socket and an HTTP stack, but no ICMP: the C++
/// skips the ping on a platform that has none, so the checks standing behind
/// it run — which is the whole of the difference, because `__RunOn` breaks on
/// `kCheckFinish` and a ping that *failed* takes the rest of the plan with it.
#[test]
fn a_host_that_cannot_ping_is_resolved_and_asked_all_the_same() {
    let longlink = link(&[("long.host", "1.2.3.4", 80)]);
    let shortlink = link(&[("short.host", "5.6.7.8", 80)]);
    let (mut ask, asked) = stub(|query| match query {
        Query::Ping { .. } => Answer::Nothing,
        _ => slow(query),
    });

    let mut core = SdtCore::new();
    core.set_http_netcheck_cgi("/netcheck");
    assert!(core.start_check(
        &longlink,
        &shortlink,
        NET_CHECK_BASIC | NET_CHECK_SHORT,
        UNUSE_TIMEOUT
    ));
    let results = core.run_checks(&mut ask, 3);

    let kinds: Vec<NetCheckType> = results
        .iter()
        .filter_map(CheckResultProfile::kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            NetCheckType::DnsCheck,
            NetCheckType::DnsCheck,
            NetCheckType::HttpCheck,
        ]
    );
    // the ping was asked for and not made, and no profile says otherwise
    assert!(matches!(asked.lock().unwrap()[0], Query::Ping { .. }));
    assert!(!results
        .iter()
        .any(|profile| profile.kind() == Some(NetCheckType::PingCheck)));
}

/// The other half of that: a ping the host *did* send, and that came back
/// failed, is what ends the run — `error_code != 0` sets `kCheckFinish`.
#[test]
fn a_ping_that_failed_ends_the_run_behind_it() {
    let longlink = link(&[("long.host", "1.2.3.4", 80)]);
    let shortlink = link(&[("short.host", "5.6.7.8", 80)]);
    let (mut ask, _) = stub(|query| match query {
        Query::Ping { .. } => Answer::Ping {
            error_code: -1,
            rtt: 4,
            status: None,
        },
        _ => slow(query),
    });

    let mut core = SdtCore::new();
    core.set_http_netcheck_cgi("/netcheck");
    assert!(core.start_check(
        &longlink,
        &shortlink,
        NET_CHECK_BASIC | NET_CHECK_SHORT,
        UNUSE_TIMEOUT
    ));
    let results = core.run_checks(&mut ask, 3);

    // the ping alone: nothing behind it was asked for
    assert_eq!(results.len(), 2);
    assert!(results
        .iter()
        .all(|profile| profile.kind() == Some(NetCheckType::PingCheck)));
}

#[test]
fn the_dns_check_records_what_the_resolve_answered() {
    let mut request = request_of(
        link(&[("long.host", "1.2.3.4", 80)]),
        CheckIPPorts::new(),
        0,
    );
    // one host, and the two addresses of the profile: a resolve that answers
    // with three is a profile with two
    let (mut ask, asked) = stub(|query| match query {
        Query::Dns { .. } => Answer::Dns {
            error_code: 0,
            rtt: 7,
            local_dns: "8.8.8.8".to_owned(),
            ips: vec![
                "1.1.1.1".to_owned(),
                "2.2.2.2".to_owned(),
                "3.3.3.3".to_owned(),
            ],
        },
        _ => Answer::Nothing,
    });

    let mut check = check_of(&request);
    assert!(check.start_do_check(NetCheckType::DnsCheck, &mut request, &mut ask, 1, ""));

    let profile = &request.checkresult_profiles[0];
    assert_eq!(profile.domain_name, "long.host");
    assert_eq!(profile.error_code, 0);
    assert_eq!(profile.rtt, 7);
    assert_eq!(profile.local_dns, "8.8.8.8");
    assert_eq!(profile.ip1, "1.1.1.1");
    assert_eq!(profile.ip2, "2.2.2.2");
    assert_eq!(request.check_status, CheckStatus::CheckContinue);

    // a request that named no timeout asks for `DEFAULT_DNS_TIMEOUT`
    assert_eq!(
        asked.lock().unwrap()[0],
        Query::Dns {
            domain: "long.host".to_owned(),
            timeout_ms: 3000
        }
    );
}

#[test]
fn a_resolve_that_failed_takes_no_address_from_the_answer() {
    let mut request = request_of(
        link(&[("long.host", "1.2.3.4", 80)]),
        CheckIPPorts::new(),
        0,
    );
    // a resolver that filled `ipinfo` in and still failed: the C++ reads the
    // addresses inside `if (0 == ret)` and nowhere else, so they are not the
    // profile's — and `DumpCheckResult` does not print an ip for a host that
    // could not be resolved
    let (mut ask, _) = stub(|query| match query {
        Query::Dns { .. } => Answer::Dns {
            error_code: -1,
            rtt: 12,
            local_dns: "8.8.8.8".to_owned(),
            ips: vec!["1.1.1.1".to_owned(), "2.2.2.2".to_owned()],
        },
        _ => Answer::Nothing,
    });

    let mut check = check_of(&request);
    assert!(check.start_do_check(NetCheckType::DnsCheck, &mut request, &mut ask, 1, ""));

    let profile = &request.checkresult_profiles[0];
    assert_eq!(profile.error_code, -1);
    assert_eq!(profile.rtt, 12);
    // the resolver that was asked is named whether or not it answered
    assert_eq!(profile.local_dns, "8.8.8.8");
    assert!(profile.ip1.is_empty(), "ip1: {}", profile.ip1);
    assert!(profile.ip2.is_empty(), "ip2: {}", profile.ip2);
    assert_eq!(request.check_status, CheckStatus::CheckFinish);
}

#[test]
fn the_tcp_check_records_the_noop_round_trip() {
    let mut request = request_of(
        link(&[("long.host", "1.2.3.4", 80)]),
        CheckIPPorts::new(),
        0,
    );
    let (mut ask, asked) = stub(slow);
    let mut check = check_of(&request);
    assert!(check.start_do_check(NetCheckType::TcpCheck, &mut request, &mut ask, 1, ""));

    let profile = &request.checkresult_profiles[0];
    assert_eq!(profile.error_code, 0);
    assert_eq!(profile.rtt, 100);
    // the connect is its own timing, and not the round trip the noop took
    assert_eq!(profile.conntime, 40);
    assert_eq!(profile.ip, "1.2.3.4");
    assert_eq!(profile.port, 80);
    assert_eq!(
        asked.lock().unwrap()[0],
        Query::Tcp {
            ip: "1.2.3.4".to_owned(),
            port: 80,
            timeout_ms: 5000
        }
    );
}

#[test]
fn a_noop_that_did_not_go_out_is_a_send_error() {
    let mut request = request_of(
        link(&[("long.host", "1.2.3.4", 80)]),
        CheckIPPorts::new(),
        0,
    );
    let (mut ask, _) = stub(|query| match query {
        // a noop that did not go out is not received on
        Query::Tcp { .. } => Answer::Tcp {
            sent: -1,
            received: 0,
            is_noop_resp: false,
            conntime: 9,
            rtt: 9,
        },
        _ => Answer::Nothing,
    });
    let mut check = check_of(&request);
    assert!(check.start_do_check(NetCheckType::TcpCheck, &mut request, &mut ask, 1, ""));

    // `kSndRcvErr`, and no round trip to report: the C++ takes `cost_time`
    // only for a round trip that happened
    let profile = &request.checkresult_profiles[0];
    assert_eq!(profile.error_code, TcpErrCode::SndRcvErr.as_i32());
    assert_eq!(profile.rtt, 0);
    assert_eq!(request.check_status, CheckStatus::CheckFinish);
}

#[test]
fn a_receive_that_failed_is_not_the_last_host_the_check_looks_at() {
    let mut request = request_of(
        link(&[("a.host", "1.1.1.1", 80), ("b.host", "2.2.2.2", 80)]),
        CheckIPPorts::new(),
        1000,
    );
    // `a.host` takes the noop and then fails the receive, and it takes longer
    // than the whole run was given; `b.host` answers
    let (mut ask, _) = stub(|query| match query {
        Query::Tcp { ip, .. } if ip == "1.1.1.1" => Answer::Tcp {
            sent: 0,
            received: -1,
            is_noop_resp: false,
            conntime: 0,
            rtt: 1200,
        },
        _ => Answer::Tcp {
            sent: 0,
            received: 0,
            is_noop_resp: true,
            conntime: 0,
            rtt: 10,
        },
    });
    let mut check = check_of(&request);
    assert!(check.start_do_check(NetCheckType::TcpCheck, &mut request, &mut ask, 1, ""));

    // two profiles, not one: the C++ `continue`s out of a receive that failed
    assert_eq!(request.checkresult_profiles.len(), 2);
    let failed = &request.checkresult_profiles[0];
    assert_eq!(failed.ip, "1.1.1.1");
    assert_eq!(failed.error_code, TcpErrCode::SndRcvErr.as_i32());
    assert_eq!(failed.rtt, 0, "no round trip to report");
    assert_eq!(request.checkresult_profiles[1].error_code, 0);

    // and neither the status the run reports nor the timeout it has left is
    // the failed receive's to decide: the 1200 ms it cost is not spent
    assert_eq!(request.check_status, CheckStatus::CheckContinue);
    assert_eq!(check.remaining(), 990);
}

#[test]
fn an_answer_that_was_not_the_noops_is_a_response_error() {
    let mut request = request_of(
        link(&[("long.host", "1.2.3.4", 80)]),
        CheckIPPorts::new(),
        0,
    );
    let (mut ask, _) = stub(|query| match query {
        Query::Tcp { .. } => Answer::Tcp {
            sent: 0,
            received: 0,
            is_noop_resp: false,
            conntime: 0,
            rtt: 9,
        },
        _ => Answer::Nothing,
    });
    let mut check = check_of(&request);
    assert!(check.start_do_check(NetCheckType::TcpCheck, &mut request, &mut ask, 1, ""));

    // `kTcpRespErr` — what came back was not the answer to the noop
    let profile = &request.checkresult_profiles[0];
    assert_eq!(profile.error_code, TcpErrCode::TcpRespErr.as_i32());
    assert_eq!(profile.rtt, 9);
    assert_eq!(request.check_status, CheckStatus::CheckFinish);
}

#[test]
fn the_ping_check_records_the_status_of_a_run_that_came_back() {
    let mut request = request_of(link(&[("long.host", "", 80)]), CheckIPPorts::new(), 5000);
    let (mut ask, asked) = stub(slow);
    let mut check = check_of(&request);
    assert!(check.start_do_check(NetCheckType::PingCheck, &mut request, &mut ask, 1, ""));

    let profile = &request.checkresult_profiles[0];
    assert_eq!(profile.error_code, 0);
    assert_eq!(profile.checkcount, DEFAULT_PING_COUNT);
    // `snprintf(loss_rate, 16, "%f", ...)`: six decimals, like `%f`
    assert_eq!(profile.loss_rate, "0.000000");
    assert_eq!(profile.rtt_str, "12.500000");
    // and `rtt_str` is the `rtt` as a string, so the two are the same number
    assert_eq!(profile.rtt, 12);
    // an item with no ip is pinged at `DEFAULT_PING_HOST`
    assert_eq!(profile.ip, DEFAULT_PING_HOST);
    // and the timeout the C++ hands `RunPingQuery` is in seconds
    assert_eq!(
        asked.lock().unwrap()[0],
        Query::Ping {
            host: DEFAULT_PING_HOST.to_owned(),
            timeout_s: 5
        }
    );
}

#[test]
fn a_ping_that_did_not_come_back_has_no_status() {
    let mut request = request_of(
        link(&[("long.host", "1.2.3.4", 80)]),
        CheckIPPorts::new(),
        0,
    );
    // a status the host filled in anyway: the C++'s `if (0 == ret)` is what
    // decides, not whether there is a status to get
    let (mut ask, _) = stub(|query| match query {
        Query::Ping { .. } => Answer::Ping {
            error_code: -1,
            rtt: 2,
            status: Some(PingStatus::new(1.0, 9.0)),
        },
        _ => Answer::Nothing,
    });
    let mut check = check_of(&request);
    assert!(check.start_do_check(NetCheckType::PingCheck, &mut request, &mut ask, 1, ""));

    let profile = &request.checkresult_profiles[0];
    assert_eq!(profile.error_code, -1);
    assert_eq!(profile.checkcount, DEFAULT_PING_COUNT);
    assert!(profile.loss_rate.is_empty());
    assert!(profile.rtt_str.is_empty());
    // a run that came back failed has no status to take a round trip from
    assert_eq!(profile.rtt, 0);
    // and it is filed under the item it was made for, which is the long-link
    // host's ip and not `DEFAULT_PING_HOST`: the ip is an empty string only
    // when the item has none
    assert_eq!(profile.ip, "1.2.3.4");
}

/// A status that is not a number: every probe lost, so the average of the
/// round trips that came back is one over none of them.
#[test]
fn a_status_that_is_not_a_number_is_left_out_of_the_report() {
    let mut request = request_of(
        link(&[("long.host", "1.2.3.4", 80)]),
        CheckIPPorts::new(),
        5000,
    );
    let (mut ask, _) = stub(|query| match query {
        Query::Ping { .. } => Answer::Ping {
            error_code: 0,
            rtt: 2,
            status: Some(PingStatus::new(f32::NAN, f32::INFINITY)),
        },
        _ => Answer::Nothing,
    });
    let mut check = check_of(&request);
    assert!(check.start_do_check(NetCheckType::PingCheck, &mut request, &mut ask, 1, ""));

    let profile = &request.checkresult_profiles[0];
    assert_eq!(profile.error_code, 0);
    assert!(profile.loss_rate.is_empty());
    assert!(profile.rtt_str.is_empty());
    assert_eq!(profile.rtt, 0);
    // `NaN` and `inf` are not JSON, and the report is one document the app
    // parses: a number that is not one in a single field of it is a whole
    // diagnosis the app cannot read.
    let json = report_json(&request.checkresult_profiles);
    assert!(!json.contains("NaN"), "{json}");
    assert!(!json.contains("inf"), "{json}");
}

#[test]
fn the_url_of_the_http_check_is_the_host_and_the_cgi() {
    let shortlink = link(&[("", "1.1.1.1", 80), ("http://short.host", "2.2.2.2", 80)]);
    let mut request = request_of(CheckIPPorts::new(), shortlink, UNUSE_TIMEOUT);
    let (mut ask, asked) = stub(slow);
    let mut check = check_of(&request);
    assert!(check.start_do_check(
        NetCheckType::HttpCheck,
        &mut request,
        &mut ask,
        1,
        "/netcheck"
    ));

    let urls: Vec<&str> = request
        .checkresult_profiles
        .iter()
        .map(|profile| profile.url.as_str())
        .collect();
    // a host with no name is `DEFAULT_HTTP_HOST`, and `http://` goes in front
    // of one that has no scheme
    assert_eq!(urls[0], format!("http://{DEFAULT_HTTP_HOST}/netcheck"));
    assert_eq!(urls[1], "http://short.host/netcheck");
    // `SendHttpQuery` gets the timeout the request has, and the default of
    // its kind when the request has none — the same reading every other probe
    // is asked under, and not `UNUSE_TIMEOUT` itself, which is twenty-four
    // days of it
    assert_eq!(
        asked.lock().unwrap()[1],
        Query::Http {
            url: "http://short.host/netcheck".to_owned(),
            timeout_ms: HTTP_DEFAULT_TIMEOUT
        }
    );
    assert_eq!(request.checkresult_profiles[0].status_code, 200);
    assert_eq!(request.checkresult_profiles[0].rtt, 100);
}

/// The budget a probe before it spent is no reason to ask the HTTP probe for
/// no time at all: `0` is a probe that cannot answer, which the check files as
/// the network being down, and not one that was given the default.
#[test]
fn an_http_check_with_nothing_left_asks_for_the_default() {
    // two hosts, one port each: `spend` breaks the port loop, and the host
    // loop walks on
    let shortlink = link(&[("short.a", "1.1.1.1", 80), ("short.b", "2.2.2.2", 80)]);
    // ten milliseconds, and every probe of `slow` takes a hundred
    let mut request = request_of(CheckIPPorts::new(), shortlink, 10);
    let (mut ask, asked) = stub(slow);
    let mut check = check_of(&request);

    assert!(check.start_do_check(
        NetCheckType::HttpCheck,
        &mut request,
        &mut ask,
        1,
        "/netcheck"
    ));
    assert_eq!(check.remaining(), 0, "the first probe spent the budget");

    let asked = asked.lock().unwrap().clone();
    assert_eq!(
        asked,
        vec![
            Query::Http {
                url: "http://short.a/netcheck".to_owned(),
                timeout_ms: 10
            },
            Query::Http {
                url: "http://short.b/netcheck".to_owned(),
                timeout_ms: HTTP_DEFAULT_TIMEOUT
            },
        ],
        "the second host is asked anyway, and not for nothing"
    );
}

#[test]
fn a_cancelled_run_stops_at_the_next_host() {
    let longlink = link(&[("long.a", "1.1.1.1", 80), ("long.b", "2.2.2.2", 80)]);
    let mut request = request_of(longlink, CheckIPPorts::new(), UNUSE_TIMEOUT);

    let cancel = CancelHandle::new();
    let cancel_in_probe = cancel.clone();
    let asked = Arc::new(Mutex::new(0));
    let sink = Arc::clone(&asked);
    let mut ask = Ask::new(move |query| {
        *sink.lock().unwrap() += 1;
        // the host that is blocked on its own socket is the one that cancels:
        // `is_canceled_` is where the C++'s `CancelCheck()` lands
        cancel_in_probe.cancel();
        match query {
            Query::Ping { .. } => Answer::Ping {
                error_code: 0,
                rtt: 3,
                status: Some(PingStatus::new(0.0, 1.0)),
            },
            _ => Answer::Nothing,
        }
    });

    let mut check = Check::new(cancel, UNUSE_TIMEOUT);
    assert!(check.start_do_check(NetCheckType::PingCheck, &mut request, &mut ask, 1, ""));
    // one host was pinged, and the second one was not
    assert_eq!(request.checkresult_profiles.len(), 1);
    assert_eq!(request.checkresult_profiles[0].ip, "1.1.1.1");
    assert_eq!(*asked.lock().unwrap(), 1);

    // and the check after it does not start either
    request.checkresult_profiles.clear();
    assert!(check.start_do_check(NetCheckType::DnsCheck, &mut request, &mut ask, 1, ""));
    assert!(request.checkresult_profiles.is_empty());
    assert_eq!(*asked.lock().unwrap(), 1);
    assert!(check.is_cancelled());

    // the two checks that walk one link only stop at their first host too
    for kind in [NetCheckType::TcpCheck, NetCheckType::HttpCheck] {
        let mut request = request_of(
            link(&[("long.a", "1.1.1.1", 80)]),
            link(&[("short.a", "2.2.2.2", 80)]),
            UNUSE_TIMEOUT,
        );
        assert!(check.start_do_check(kind, &mut request, &mut ask, 1, ""));
        assert!(request.checkresult_profiles.is_empty(), "{kind:?}");
    }
    assert_eq!(*asked.lock().unwrap(), 1);
}

#[test]
fn a_timeout_that_runs_out_is_spent_down_to_nothing() {
    let longlink = link(&[("long.a", "1.1.1.1", 80), ("long.b", "2.2.2.2", 80)]);
    let shortlink = link(&[("short.a", "3.3.3.3", 80), ("short.b", "4.4.4.4", 80)]);
    let mut request = request_of(longlink.clone(), shortlink.clone(), 10);
    // every probe takes longer than the 10 ms the run was given
    let (mut ask, asked) = stub(slow);

    let mut check = check_of(&request);
    assert!(check.start_do_check(NetCheckType::DnsCheck, &mut request, &mut ask, 1, ""));
    // the C++ `break`s out of the loop it is in only, so the short-link hosts
    // are asked anyway: one result per link, and no more
    assert_eq!(request.checkresult_profiles.len(), 2);
    assert_eq!(request.checkresult_profiles[0].domain_name, "long.a");
    assert_eq!(request.checkresult_profiles[1].domain_name, "short.a");
    assert_eq!(asked.lock().unwrap().len(), 2);

    // and the next check does not run at all: `total_timeout <= 0`
    assert!(!check.start_do_check(NetCheckType::HttpCheck, &mut request, &mut ask, 1, ""));
    assert_eq!(request.check_status, CheckStatus::CheckFinish);
    assert_eq!(check.remaining(), 0);
    assert_eq!(asked.lock().unwrap().len(), 2);

    // every other check spends the same timeout the same way: the host it was
    // spent on is reported, and so is every host behind it — the C++ `break`s
    // out of the loop over the ip/port of one name and not out of the walk —
    // and the ping walks both links
    for kind in [
        NetCheckType::PingCheck,
        NetCheckType::TcpCheck,
        NetCheckType::HttpCheck,
    ] {
        let mut request = request_of(longlink.clone(), shortlink.clone(), 10);
        let mut check = check_of(&request);
        assert!(check.start_do_check(kind, &mut request, &mut ask, 1, "/netcheck"));
        assert_eq!(check.remaining(), 0, "{kind:?}");
        let walked = if kind == NetCheckType::PingCheck {
            4
        } else {
            2
        };
        assert_eq!(request.checkresult_profiles.len(), walked, "{kind:?}");
    }

    // `CancelDoCheck()`, which the C++'s destructor calls
    check.cancel();
    assert!(check.is_cancelled());
}

#[test]
fn a_spent_timeout_ends_the_name_it_was_spent_on_and_not_the_walk() {
    // two ip/port under one name, and one name behind it
    let longlink: CheckIPPorts = [
        (
            "long.a".to_owned(),
            vec![
                CheckIPPort::new("1.1.1.1", 80),
                CheckIPPort::new("2.2.2.2", 80),
            ],
        ),
        ("long.b".to_owned(), vec![CheckIPPort::new("3.3.3.3", 80)]),
    ]
    .into_iter()
    .collect();
    let mut request = request_of(longlink, CheckIPPorts::new(), 10);
    // every probe takes longer than the 10 ms the run was given
    let (mut ask, asked) = stub(slow);
    let mut check = check_of(&request);

    assert!(check.start_do_check(NetCheckType::TcpCheck, &mut request, &mut ask, 1, ""));

    // the first ip of the name that spent it, and then the name behind it: not
    // the second ip of the name that spent it, because the `break` the C++
    // takes is the inner loop's
    let ips: Vec<String> = request
        .checkresult_profiles
        .iter()
        .map(|profile| profile.ip.clone())
        .collect();
    assert_eq!(ips, vec!["1.1.1.1".to_owned(), "3.3.3.3".to_owned()]);
    assert_eq!(asked.lock().unwrap().len(), 2);
}

#[test]
fn the_kinds_without_a_checker_check_nothing() {
    let mut request = request_of(
        link(&[("long.host", "1.2.3.4", 80)]),
        link(&[("short.host", "5.6.7.8", 80)]),
        UNUSE_TIMEOUT,
    );
    let (mut ask, asked) = stub(slow);
    let mut check = check_of(&request);

    for kind in [
        NetCheckType::NewDnsCheck,
        NetCheckType::TracerouteCheck,
        NetCheckType::ReqBufCheck,
    ] {
        assert!(check.start_do_check(kind, &mut request, &mut ask, 1, "/netcheck"));
    }
    // the C++ has no `NewDnsChecker`, no `TracerouteChecker` and no
    // `ReqBufChecker`, so nothing was asked and nothing was recorded
    assert!(request.checkresult_profiles.is_empty());
    assert!(asked.lock().unwrap().is_empty());
    assert_eq!(request.check_status, CheckStatus::CheckContinue);
}

/// Keeps what the logic reported, so the test can look at it afterwards.
struct Report(Arc<Mutex<Vec<CheckResultProfile>>>);

impl Callback for Report {
    fn report_net_check_result(&self, check_results: &[CheckResultProfile]) {
        self.0.lock().unwrap().extend_from_slice(check_results);
    }
}

#[test]
fn a_diagnosis_runs_through_the_core_and_the_logic() {
    let longlink = link(&[("long.host", "1.2.3.4", 80)]);
    let shortlink = link(&[("short.host", "5.6.7.8", 80)]);

    let mut core = SdtCore::new();
    core.set_http_netcheck_cgi("/netcheck");
    assert!(core.start_check(
        &longlink,
        &shortlink,
        NET_CHECK_BASIC | NET_CHECK_SHORT,
        UNUSE_TIMEOUT
    ));

    let (mut ask, _) = stub(slow);
    let results = core.run_checks(&mut ask, 3);
    let kinds: Vec<NetCheckType> = results
        .iter()
        .filter_map(CheckResultProfile::kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            NetCheckType::PingCheck,
            NetCheckType::PingCheck,
            NetCheckType::DnsCheck,
            NetCheckType::DnsCheck,
            NetCheckType::HttpCheck,
        ]
    );
    // `comm::getNetInfo()`, which the host hands the run
    assert_eq!(results[0].network_type, 3);

    // the same through `SdtLogic`, which is what the app calls
    let mut logic = SdtLogic::new();
    let reported = Arc::new(Mutex::new(Vec::new()));
    logic.set_callback(Report(Arc::clone(&reported)));
    logic.set_http_netcheck_cgi("/netcheck");
    assert!(logic.start_active_check(
        &longlink,
        &shortlink,
        NET_CHECK_BASIC | NET_CHECK_SHORT,
        UNUSE_TIMEOUT
    ));
    let results = logic.run_checks(&mut ask, 3);
    assert_eq!(results.len(), 5);
    // `ReportNetCheckResult` is what `run` does that `run_on` does not
    assert_eq!(reported.lock().unwrap().len(), 5);
}

/// The walk goes on after the budget is spent — the C++ `break`s out of the
/// loop it is in and no more — but a probe is not asked for with nothing:
/// `0` milliseconds is a resolve or a connect that cannot answer, which the
/// check files as the network being down. It gets the default instead.
#[test]
fn a_probe_asked_for_with_nothing_left_is_asked_with_the_default() {
    let longlink = link(&[("long.a", "1.1.1.1", 80)]);
    let shortlink = link(&[("short.a", "3.3.3.3", 80)]);
    let mut request = request_of(longlink, shortlink, 10);
    let (mut ask, asked) = stub(slow);

    let mut check = check_of(&request);
    assert!(check.start_do_check(NetCheckType::DnsCheck, &mut request, &mut ask, 1, ""));
    let asked = asked.lock().unwrap().clone();
    assert_eq!(
        asked,
        vec![
            Query::Dns {
                domain: "long.a".to_owned(),
                timeout_ms: 10
            },
            Query::Dns {
                domain: "short.a".to_owned(),
                timeout_ms: DEFAULT_DNS_TIMEOUT
            },
        ],
        "the short-link host is asked anyway, and not for nothing"
    );
}
