//! Cache lifecycle on real open private TLS/IP sessions.
use super::super::super::types::{DirectAccessState, DirectGrantState};
use super::*;

#[test]
fn review_task_s09_transport_path_revisions_reject_returned_path_evidence() {
    let mut fixture = Fixture::start(30);
    let rt = fixture.a.rt.clone();
    rt.block_on(async {
        let (connection, fact) = fixture.positive().await;
        let transport = &fixture.a.lan_links;
        let path = selected_ip_path(&connection).unwrap();
        let revision = transport.path_revision(&connection).unwrap();
        // Deterministic away/return events, not physical OS-NIC migration.
        transport.path_changed(&connection);
        assert!(fixture.a.lan_link_snapshot().is_empty());
        transport.path_changed(&connection);
        assert_eq!(transport.path_revision(&connection).unwrap(), revision + 2);
        assert!(selected_ip_path(&connection).as_ref() == Some(&path));
        assert!(
            transport
                .confirm(
                    &fixture.a,
                    &connection,
                    fact.pin,
                    path,
                    revision,
                    fact.challenge,
                    OwnUplink::Absent,
                )
                .is_err(),
            "returned addresses cannot resurrect old confirmation"
        );
        assert!(fixture.a.lan_link_snapshot().is_empty());
        fixture.start_rounds(&connection, fixture.expected_pin());
        let (new, _) = fixture.wait_status().await;
        assert_ne!(
            new.challenge, fact.challenge,
            "only a new real exchange restores status"
        );
    });
}

#[test]
fn review_task_s09_transport_close_withdraw_disable_discard_cached_facts() {
    for mode in 0..4 {
        let mut fixture = Fixture::start(40 + mode * 2);
        let rt = fixture.a.rt.clone();
        rt.block_on(async {
            let (connection, fact) = fixture.positive().await;
            let transport = &fixture.a.lan_links;
            let path = selected_ip_path(&connection).unwrap();
            let revision = transport.path_revision(&connection).unwrap();
            match mode {
                0 => close(&connection),
                1 => {
                    fixture.a.auth.lock().unwrap().direct_contacts[0].access_state =
                        DirectAccessState::Ignored;
                }
                2 => {
                    fixture.b.auth.lock().unwrap().direct_grants[0].state =
                        DirectGrantState::Reconfirm;
                    assert!(fixture.b.lan_link_snapshot().is_empty());
                    wait_until("withdrawn grant closes channel", || {
                        connection.close_reason().is_some()
                    })
                    .await;
                }
                _ => {
                    let state = transport.state.lock().unwrap();
                    transport.disable(); // force the nonblocking cleanup fallback
                    drop(state);
                    transport
                        .update(fixture.lan.host(OwnUplink::Present))
                        .unwrap();
                }
            }
            assert!(fixture.a.lan_link_snapshot().is_empty());
            assert!(transport
                .confirm(
                    &fixture.a,
                    &connection,
                    fact.pin,
                    path,
                    revision,
                    fact.challenge,
                    OwnUplink::Absent,
                )
                .is_err());
            assert_status_only(&fixture.a, &connection, fixture.baseline[0]);
        });
    }
}

#[test]
fn review_task_s09_transport_expired_or_unknown_facts_fail_closed() {
    for mode in 0..4 {
        let mut fixture = Fixture::start(60 + mode * 2);
        let rt = fixture.a.rt.clone();
        rt.block_on(async {
            let (connection, _) = fixture.positive().await;
            match mode {
                0 => {
                    fixture
                        .a
                        .lan_links
                        .state
                        .lock()
                        .unwrap()
                        .cache
                        .get_mut(&connection.stable_id())
                        .unwrap()
                        .confirmed = Instant::now() - FACT_MONOTONIC_TTL - Duration::from_secs(1);
                }
                1 => {
                    fixture
                        .a
                        .lan_links
                        .state
                        .lock()
                        .unwrap()
                        .cache
                        .get_mut(&connection.stable_id())
                        .unwrap()
                        .fact
                        .expires_at = now_secs();
                }
                2 => {
                    fixture
                        .a
                        .lan_links
                        .state
                        .lock()
                        .unwrap()
                        .host
                        .as_mut()
                        .unwrap()
                        .1 = Instant::now() - HOST_FACT_TTL - Duration::from_secs(1);
                }
                _ => {
                    let previous = fixture.b.lan_link_snapshot().remove(0).challenge;
                    fixture
                        .b
                        .update_lan_link_host(fixture.lan.host(OwnUplink::Unknown))
                        .unwrap();
                    fixture.start_rounds(&connection, fixture.expected_pin());
                    wait_until("Unknown delivered through a complete real round", || {
                        fixture
                            .b
                            .lan_link_snapshot()
                            .iter()
                            .any(|fact| fact.challenge != previous)
                    })
                    .await;
                    wait_until("Unknown cleared old status authority", || {
                        fixture.a.lan_link_snapshot().is_empty()
                    })
                    .await;
                    assert!(
                        !fixture.rounds.as_ref().unwrap().is_finished(),
                        "Unknown keeps the status session usable"
                    );
                }
            }
            assert!(
                fixture.a.lan_link_snapshot().is_empty(),
                "unknown/expired status is never no-uplink authority"
            );
            assert_status_only(&fixture.a, &connection, fixture.baseline[0]);
        });
    }
}
