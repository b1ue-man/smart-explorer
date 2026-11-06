use super::*;

const LOCAL: &str = "rechte_task_local";

fn peer(key: &str) -> OutgoingPeer<'_> {
    OutgoingPeer {
        local_node: "rechte_task_local",
        key,
        node_id: key,
        device_id: "device",
        device_name: "Laptop",
    }
}

#[test]
fn rechte_task_first_rejection_is_not_paused_then_pauses_grow() {
    let key = "rechte_task_pause_growth";
    record_outgoing(
        peer(key),
        Some(SessionDenial::NotGranted),
        "nicht freigegeben",
    );
    assert!(
        outgoing_paused(LOCAL, key).is_none(),
        "the first rejection is retried"
    );
    record_outgoing(
        peer(key),
        Some(SessionDenial::NotGranted),
        "nicht freigegeben",
    );
    let paused = outgoing_paused(LOCAL, key).expect("second rejection pauses");
    assert_eq!(paused.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(paused.to_string(), "nicht freigegeben");
    assert_eq!(
        super::super::denial::peer_denial_of(&paused),
        Some(Some(SessionDenial::NotGranted))
    );
    let row = rows(LOCAL)
        .into_iter()
        .find(|row| row.node_id == key)
        .expect("outgoing row");
    assert_eq!(row.direction, "outgoing");
    assert_eq!(row.count, 2);
    assert!(row.retry_in_secs.is_some_and(|secs| secs <= 5));
    clear_outgoing(LOCAL, key);
    assert!(outgoing_paused(LOCAL, key).is_none());
}

#[test]
fn rechte_task_presence_ends_the_pause_but_keeps_the_count() {
    let key = "rechte_task_presence";
    for _ in 0..3 {
        record_outgoing(peer(key), None, "Share-Anmeldung nicht autorisiert");
    }
    assert!(outgoing_paused(LOCAL, key).is_some());
    forget_outgoing_node(LOCAL, key);
    assert!(outgoing_paused(LOCAL, key).is_none());
    record_outgoing(peer(key), None, "Share-Anmeldung nicht autorisiert");
    assert!(outgoing_paused(LOCAL, key).is_some(), "the count continues");
    clear_outgoing(LOCAL, key);
}

#[test]
fn rechte_task_incoming_rejections_are_counted_and_cleared() {
    let device = "rechte_task_incoming_device";
    record_incoming(device, "key", "node", "direct", SessionDenial::NotGranted);
    record_incoming(device, "key", "node", "direct", SessionDenial::Reconfirm);
    let row = rows(LOCAL)
        .into_iter()
        .find(|row| row.device_id == device)
        .expect("incoming row");
    assert_eq!(row.direction, "incoming");
    assert_eq!(row.count, 2);
    assert_eq!(row.denial, Some(SessionDenial::Reconfirm));
    clear_incoming(device, "key");
    assert!(rows(LOCAL).into_iter().all(|row| row.device_id != device));
}
