use std::net::SocketAddr;

use super::*;
use crate::tracked_direct;

fn addr(text: &str) -> SocketAddr {
    text.parse().unwrap()
}

/// User 2026-11-25 („TCP und UDP, DIREKT“): the TCP reflector and the STUN
/// responder listen beside the signaling IP by default (dual-stack for
/// `0.0.0.0`), `off` or an invalid value turns one off, `[::]` also takes
/// `0.0.0.0` where the OS keeps the two apart, and `hello_ok` names a
/// running service bare on its default port, with its port otherwise, and
/// not at all while it does not run.
#[test]
fn address_services_task_plans_fallback_and_hello_announcement() {
    let env = "SE_SHARE_TCP_REFLECT_BIND";
    let any = addr("0.0.0.0:51820");
    let rows = [
        (
            "default",
            None,
            any,
            ServicePlan::Listen(addr("[::]:51823")),
        ),
        (
            "empty",
            Some(" "),
            any,
            ServicePlan::Listen(addr("[::]:51823")),
        ),
        (
            "fixed signaling IP",
            None,
            addr("198.51.100.4:51820"),
            ServicePlan::Listen(addr("198.51.100.4:51823")),
        ),
        (
            "explicit",
            Some("192.0.2.1:6000"),
            any,
            ServicePlan::Listen(addr("192.0.2.1:6000")),
        ),
        (
            "off",
            Some("OFF"),
            any,
            ServicePlan::Off(format!("disabled via {env}=off")),
        ),
    ];
    for (case, value, signal, expected) in rows {
        assert_eq!(plan(env, value, signal, 51823), expected, "{case}");
    }
    match plan(env, Some("nonsense"), any, 51823) {
        ServicePlan::Off(reason) => {
            assert!(reason.starts_with("invalid SE_SHARE_TCP_REFLECT_BIND"))
        }
        listen => panic!("an invalid value listens: {listen:?}"),
    }

    // Binds of `[::]`: dual-stack Linux refuses the extra `0.0.0.0`,
    // Windows takes it, a host without IPv6 gets `0.0.0.0` alone.
    let v6 = addr("[::]:3478");
    let v4 = addr("0.0.0.0:3478");
    let refuse = |refused: SocketAddr| {
        move |address: SocketAddr| {
            if address == refused {
                Err(std::io::Error::from(std::io::ErrorKind::AddrInUse))
            } else {
                Ok(address)
            }
        }
    };
    let binds = [
        ("dual-stack", refuse(v4), vec![v6]),
        ("IPv6 only", refuse(addr("192.0.2.9:1")), vec![v6, v4]),
        ("no IPv6", refuse(v6), vec![v4]),
    ];
    for (case, bind, expected) in binds {
        assert_eq!(bind_all(v6, bind), Ok(expected), "{case}");
    }
    assert_eq!(
        bind_all(addr("192.0.2.1:3478"), refuse(v4)),
        Ok(vec![addr("192.0.2.1:3478")])
    );
    assert!(
        bind_all(addr("192.0.2.1:3478"), refuse(addr("192.0.2.1:3478")))
            .unwrap_err()
            .starts_with("cannot bind 192.0.2.1:3478")
    );

    let announced = [
        (
            "reflector default",
            "tcp_reflect_v1",
            Some(51823),
            None,
            Some("tcp_reflect_v1"),
        ),
        (
            "reflector port",
            "tcp_reflect_v1",
            Some(51900),
            None,
            Some("tcp_reflect_v1:51900"),
        ),
        ("reflector off", "tcp_reflect_v1", None, Some(3478), None),
        ("STUN default", "stun_v1", None, Some(3478), Some("stun_v1")),
        (
            "STUN port",
            "stun_v1",
            Some(51823),
            Some(3479),
            Some("stun_v1:3479"),
        ),
        (
            "other",
            "tracked_direct_v1",
            None,
            None,
            Some("tracked_direct_v1"),
        ),
    ];
    for (case, capability, reflect, stun_port, expected) in announced {
        assert_eq!(
            announce_with(capability, reflect, stun_port).as_deref(),
            expected,
            "{case}"
        );
    }

    // Negotiation keeps both offered capabilities; a server whose services
    // do not run (as in this test process) does not list them.
    let negotiated = tracked_direct::negotiate_capabilities(vec![
        "tcp_reflect_v1".into(),
        "stun_v1".into(),
        "tracked_direct_v1".into(),
    ]);
    assert!(negotiated.contains("tcp_reflect_v1") && negotiated.contains("stun_v1"));
    assert_eq!(
        tracked_direct::capability_list(&negotiated),
        ["tracked_direct_v1"]
    );
}
