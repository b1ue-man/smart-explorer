use std::net::{IpAddr, SocketAddr};

use super::*;
use crate::net::InterfaceFacts;
use crate::screen::core::wire::{tag_limit, MAX_CONTROL_BYTES};
use crate::screen::core::wire_hello::tests::server_hello;
use crate::screen::core::wire_hello::ScreenServerHello;

fn addr(text: &str) -> SocketAddr {
    text.parse().unwrap()
}

fn local(text: &str) -> TcpCandidate {
    TcpCandidate {
        addr: addr(text),
        kind: CandidateKind::Local,
    }
}

fn answer(candidates: Vec<TcpCandidate>) -> VideoTcpAnswer {
    VideoTcpAnswer {
        candidates,
        token: TcpToken([7; TOKEN_BYTES]),
        cert_sha256: [0xab; 32],
    }
}

/// User 2026-11-25 („TCP und UDP, DIREKT“): the viewer's offer (240), the
/// host's answer (241) and the result (242) travel as JSON on the control
/// stream within the default budget; a peer's candidates are checked and
/// canonicalized, the token and key hash are exactly 32 bytes of hex, and a
/// host hello without `video_tcp` stays as older viewers read it.
#[test]
fn video_tcp_task_frames_round_trip_and_refuse_bad_candidates() {
    let offer = VideoTcpOffer {
        candidates: vec![local("192.168.1.5:40000"), local("[2001:db8::5]:40001")],
    };
    let json = String::from_utf8(offer.encode().unwrap()).unwrap();
    assert_eq!(
        json,
        r#"{"candidates":[{"addr":"192.168.1.5:40000","kind":"local"},{"addr":"[2001:db8::5]:40001","kind":"local"}]}"#
    );
    assert_eq!(VideoTcpOffer::decode(json.as_bytes()).unwrap(), offer);

    let sent = answer(vec![TcpCandidate {
        addr: addr("203.0.113.7:51000"),
        kind: CandidateKind::Reflected,
    }]);
    let decoded = VideoTcpAnswer::decode(&sent.encode().unwrap()).unwrap();
    assert_eq!(decoded.candidates, sent.candidates);
    assert_eq!(decoded.token.0, [7; TOKEN_BYTES]);
    assert_eq!(decoded.cert_sha256, [0xab; 32]);
    assert_eq!(format!("{:?}", decoded.token), "TcpToken(..)");

    for result in [
        VideoTcpResult::Connected {
            remote: addr("203.0.113.7:51000"),
            path: TcpPathKind::Punched,
        },
        VideoTcpResult::Failed {
            reason: TcpFailureReason::WrongToken,
        },
    ] {
        assert_eq!(
            VideoTcpResult::decode(&result.encode().unwrap()).unwrap(),
            result
        );
    }
    assert_eq!(
        String::from_utf8(
            VideoTcpResult::Failed {
                reason: TcpFailureReason::Unreachable
            }
            .encode()
            .unwrap()
        )
        .unwrap(),
        r#"{"outcome":"failed","reason":"unreachable"}"#
    );
    // A newer peer's kind, path or reason stays readable.
    let newer = br#"{"candidates":[{"addr":"10.0.0.2:1","kind":"relay_hint"}]}"#;
    assert_eq!(
        VideoTcpOffer::decode(newer).unwrap().candidates[0].kind,
        CandidateKind::Other
    );
    let newer = br#"{"outcome":"failed","reason":"quantum"}"#;
    assert_eq!(
        VideoTcpResult::decode(newer).unwrap(),
        VideoTcpResult::Failed {
            reason: TcpFailureReason::Other
        }
    );

    // Rows: candidates a peer must not name.
    let refused = [
        ("port 0", "192.168.1.5:0"),
        ("loopback", "127.0.0.1:4000"),
        ("unspecified", "0.0.0.0:4000"),
        ("multicast", "224.0.0.1:4000"),
        ("broadcast", "255.255.255.255:4000"),
        ("IPv6 loopback", "[::1]:4000"),
        ("IPv6 link-local", "[fe80::1]:4000"),
    ];
    for (case, text) in refused {
        let offer = VideoTcpOffer {
            candidates: vec![local(text)],
        };
        let payload = serde_json::to_vec(&offer).unwrap();
        assert!(VideoTcpOffer::decode(&payload).is_err(), "{case}");
        assert!(offer.encode().is_err(), "{case}");
    }
    let many: Vec<_> = (1..=17)
        .map(|n| local(&format!("10.0.0.{n}:4000")))
        .collect();
    let payload = serde_json::to_vec(&VideoTcpOffer { candidates: many }).unwrap();
    assert!(VideoTcpOffer::decode(&payload).is_err(), "more than 16");
    // An IPv4-mapped address becomes IPv4; a repeated one is dropped.
    let payload = br#"{"candidates":[{"addr":"[::ffff:192.168.1.5]:4000","kind":"local"},{"addr":"192.168.1.5:4000","kind":"reflected"}]}"#;
    assert_eq!(
        VideoTcpOffer::decode(payload).unwrap().candidates,
        vec![local("192.168.1.5:4000")]
    );
    // Token and key hash: 64 hex digits each.
    for (case, token, hash) in [
        ("short token", "07".repeat(31), "ab".repeat(32)),
        ("not hex", "zz".repeat(32), "ab".repeat(32)),
        ("long hash", "07".repeat(32), "ab".repeat(33)),
    ] {
        let payload = serde_json::json!({ "candidates": [], "token": token, "cert_sha256": hash });
        let payload = serde_json::to_vec(&payload).unwrap();
        assert!(VideoTcpAnswer::decode(&payload).is_err(), "{case}");
    }

    // The tags sit in the default budget; the largest answer fits it.
    for tag in [VIDEO_TCP_OFFER, VIDEO_TCP_ANSWER, VIDEO_TCP_RESULT] {
        assert_eq!(tag_limit(tag), MAX_CONTROL_BYTES);
    }
    let widest: Vec<_> = (1..=MAX_TCP_CANDIDATES)
        .map(|n| local(&format!("[2001:db8:ffff:ffff:ffff:ffff:ffff:{n:x}]:65535")))
        .collect();
    assert!(answer(widest).encode().unwrap().len() < MAX_CONTROL_BYTES);

    // The hello: absent while `None`, `video_tcp: 1` once set, an older
    // hello without the field reads as `None`.
    let mut hello = server_hello();
    assert_eq!(hello.video_tcp, None);
    assert!(!serde_json::to_string(&hello).unwrap().contains("video_tcp"));
    hello.video_tcp = Some(VIDEO_TCP_V1);
    let json = serde_json::to_value(&hello).unwrap();
    assert_eq!(json["video_tcp"], 1);
    let mut older = json.clone();
    older.as_object_mut().unwrap().remove("video_tcp");
    let older: ScreenServerHello = serde_json::from_value(older).unwrap();
    assert_eq!(older.video_tcp, None);
}

fn face(name: &str, up: bool, loopback: bool, addrs: &[&str]) -> InterfaceFacts {
    InterfaceFacts {
        name: name.into(),
        adapter_id: name.into(),
        index: 1,
        up,
        loopback,
        addrs: addrs
            .iter()
            .map(|text| text.parse::<IpAddr>().unwrap())
            .collect(),
        has_gateway: false,
        dhcp_lease: None,
    }
}

/// Candidate collection: no loopback interface or address, no interface
/// that is down, no IPv6 link-local address (its scope is the dialer's), the
/// IPv4 link-local one kept (same link), LAN IPv4 first, each family with
/// its own listener port, at most `MAX_LOCAL_CANDIDATES`.
#[test]
fn video_tcp_task_local_candidates_skip_loopback_and_link_local_v6() {
    let interfaces = [
        face("lo", true, true, &["127.0.0.1", "::1"]),
        face(
            "eth0",
            true,
            false,
            &[
                "203.0.113.9",
                "192.168.1.5",
                "fe80::1",
                "2001:db8::5",
                "fd00::5",
            ],
        ),
        face("wlan0", false, false, &["10.0.0.2"]),
        face(
            "usb0",
            true,
            false,
            &["169.254.3.4", "192.168.1.5", "127.0.0.2"],
        ),
    ];
    let candidates = local_candidates(&interfaces, Some(40000), Some(40001));
    let addrs: Vec<String> = candidates.iter().map(|c| c.addr.to_string()).collect();
    assert_eq!(
        addrs,
        [
            "192.168.1.5:40000",
            "169.254.3.4:40000",
            "203.0.113.9:40000",
            "[2001:db8::5]:40001",
            "[fd00::5]:40001",
        ]
    );
    assert!(candidates.iter().all(|c| c.kind == CandidateKind::Local));
    // Without an IPv6 listener no IPv6 candidate, without IPv4 none of IPv4.
    assert_eq!(local_candidates(&interfaces, Some(40000), None).len(), 3);
    assert_eq!(local_candidates(&interfaces, None, Some(40001)).len(), 2);
    let crowded: Vec<String> = (1..=20).map(|n| format!("10.1.0.{n}")).collect();
    let crowded: Vec<&str> = crowded.iter().map(String::as_str).collect();
    let crowded = [face("br0", true, false, &crowded)];
    assert_eq!(
        local_candidates(&crowded, Some(1), None).len(),
        MAX_LOCAL_CANDIDATES
    );
}

/// The predicted candidate is the endpoint's public IPv4 with the
/// listener's port; the merged list keeps the first kind of an address and
/// at most `MAX_TCP_CANDIDATES`.
#[test]
fn video_tcp_task_predicted_and_merged_candidates() {
    let endpoint = [
        addr("192.168.1.5:1234"),
        addr("203.0.113.7:5555"),
        addr("[2001:db8::1]:1"),
        addr("100.64.0.1:3"),
        addr("[::ffff:198.51.100.2]:4"),
        addr("203.0.113.7:6666"),
    ];
    assert_eq!(
        predicted_candidates(&endpoint, Some(40000)),
        vec![addr("203.0.113.7:40000"), addr("198.51.100.2:40000")]
    );
    assert!(predicted_candidates(&endpoint, None).is_empty());

    let merged = merge_candidates(
        vec![local("192.168.1.5:40000")],
        Some(addr("203.0.113.7:40000")),
        predicted_candidates(&endpoint, Some(40000)),
    );
    let kinds: Vec<(String, CandidateKind)> = merged
        .iter()
        .map(|c| (c.addr.to_string(), c.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("192.168.1.5:40000".to_string(), CandidateKind::Local),
            ("203.0.113.7:40000".to_string(), CandidateKind::Reflected),
            ("198.51.100.2:40000".to_string(), CandidateKind::Predicted),
        ]
    );
    let many: Vec<_> = (1..=MAX_LOCAL_CANDIDATES)
        .map(|n| local(&format!("10.0.0.{n}:1")))
        .collect();
    let predicted: Vec<_> = (1..=8)
        .map(|n| addr(&format!("198.51.100.{n}:1")))
        .collect();
    let merged = merge_candidates(many, Some(addr("203.0.113.7:1")), predicted);
    assert_eq!(merged.len(), MAX_TCP_CANDIDATES);
    assert_eq!(merged[MAX_LOCAL_CANDIDATES].kind, CandidateKind::Reflected);
}

/// Path classes for the statistics and the German reasons of the status
/// line; every reason has its own text.
#[test]
fn video_tcp_task_path_classification_and_german_reasons() {
    let cases = [
        ("LAN", "192.168.1.5:1", "192.168.1.9:2", TcpPathKind::Lan),
        ("loopback", "127.0.0.1:1", "127.0.0.1:2", TcpPathKind::Lan),
        ("shared", "10.0.0.1:1", "100.70.0.1:2", TcpPathKind::Lan),
        ("ULA", "[fd00::1]:1", "[fd00::2]:2", TcpPathKind::Lan),
        (
            "IPv6",
            "[2001:db8::1]:1",
            "[2001:db8::2]:2",
            TcpPathKind::Ipv6,
        ),
        (
            "public",
            "198.51.100.1:1",
            "203.0.113.7:2",
            TcpPathKind::Public,
        ),
        (
            "punched",
            "192.168.1.5:1",
            "203.0.113.7:2",
            TcpPathKind::Punched,
        ),
        (
            "mapped",
            "[::ffff:10.0.0.1]:1",
            "[::ffff:203.0.113.7]:2",
            TcpPathKind::Punched,
        ),
    ];
    for (case, local_addr, remote, expected) in cases {
        assert_eq!(
            classify_path(addr(local_addr), addr(remote)),
            expected,
            "{case}"
        );
    }
    assert!(!is_public_v4("100.64.0.1".parse().unwrap()));
    assert!(!is_public_v4("240.0.0.1".parse().unwrap()));
    assert!(is_public_v4("8.8.8.8".parse().unwrap()));

    let reasons = [
        TcpFailureReason::Unreachable,
        TcpFailureReason::Refused,
        TcpFailureReason::WrongToken,
        TcpFailureReason::Tls,
        TcpFailureReason::Timeout,
        TcpFailureReason::LocalSocket,
        TcpFailureReason::Other,
    ];
    let texts: std::collections::HashSet<&str> = reasons.iter().map(|r| r.text()).collect();
    assert_eq!(texts.len(), reasons.len());
    assert!(TcpFailureReason::Unreachable.text().contains("nicht durch"));
    assert!(TcpFailureReason::WrongToken
        .text()
        .contains("falscher Sitzungsschlüssel"));
    assert!(TcpFailureReason::Timeout
        .text()
        .starts_with("Zeitüberschreitung"));
    assert_eq!(
        serde_json::to_string(&TcpFailureReason::WrongToken).unwrap(),
        r#""wrong_token""#
    );
    assert_eq!(TcpPathKind::Punched.label(), "durch NAT gelocht");
}
