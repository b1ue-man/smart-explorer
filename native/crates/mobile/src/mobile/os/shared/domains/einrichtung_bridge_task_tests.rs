//! Task tests of WP3 (design 10.3, U5) for the Android bridge: `tcp.start`
//! needs neither `acknowledgeLocalAccess` nor `lifetimeSecs`, a shared
//! service is opened by its port alone, and
//! `rdp.open` without a usable local RDP program reports the missing client
//! without opening anything. Prefix `einrichtung_task_`.
use super::*;

#[test]
fn einrichtung_task_bridge_tcp_start_without_acknowledgement_or_lifetime() {
    let request = start_request(&json!({"targetPort": 5432})).unwrap();
    assert!(request.acknowledge_local_access);
    assert_eq!(request.lifetime_secs, 0);
    assert_eq!(request.local_port, 0);
    let legacy = start_request(&json!({
        "targetPort": 80,
        "targetHost": "192.168.1.20",
        "lifetimeSecs": 900,
        "acknowledgeLocalAccess": false
    }))
    .unwrap();
    assert!(
        legacy.acknowledge_local_access,
        "the old flag changes nothing"
    );
    assert_eq!(legacy.lifetime_secs, 900);
    assert!(
        start_request(&json!({})).is_err(),
        "targetPort stays required"
    );
    assert_eq!(services::port_arg(&json!({"port": 8080})).unwrap(), 8080);
    assert!(services::port_arg(&json!({"port": 0})).is_err());
    assert!(services::port_arg(&json!({"port": 70000})).is_err());
}

#[test]
fn einrichtung_task_bridge_rdp_open_reports_missing_client() {
    let mut opened = false;
    let result = super::super::rdp::open_with(
        &json!({"deviceId": "device-1", "publicKey": "key-1"}),
        &[],
        |_, _, _| {
            opened = true;
            Err("must not open".into())
        },
    );
    let error = result.expect_err("no client, no start");
    assert_eq!(error.message, super::super::rdp::NO_CLIENT);
    assert!(!opened);
}
