//! R-ZUGANG-03 (b) and (d), G15 (suite-2-entdeckung): the incoming half of
//! the rejection book keeps one row per TLS-bound node, and rows of nodes no
//! relation of this host pins hold at most `MAX_UNPINNED` places and never
//! push out the rows of known devices.
//!
//! The book is process-wide; every test empties its incoming half before and
//! after (the suites run these tests one at a time).
use super::*;

const LOCAL: &str = "entdeckung_task_local";

fn incoming() -> Vec<DenialRow> {
    rows(LOCAL)
        .into_iter()
        .filter(|row| row.direction == "incoming")
        .collect()
}

fn clear_all_incoming() {
    for row in incoming() {
        clear_incoming(
            &row.device_id,
            row.public_key.as_deref().unwrap_or_default(),
        );
    }
}

struct CleanBook;

impl CleanBook {
    fn new() -> Self {
        clear_all_incoming();
        Self
    }
}

impl Drop for CleanBook {
    fn drop(&mut self) {
        clear_all_incoming();
    }
}

fn record(device: &str, node: &str, denial: SessionDenial, pinned: bool) {
    record_incoming_from(
        device,
        &format!("key-of-{node}"),
        node,
        "direct",
        denial,
        pinned,
    );
}

fn count_with_prefix(prefix: &str) -> usize {
    incoming()
        .iter()
        .filter(|row| row.node_id.starts_with(prefix))
        .count()
}

#[test]
fn entdeckung_task_one_row_per_tls_node_keeps_the_latest_claimed_device() {
    let _book = CleanBook::new();
    let node = "g15a-node";
    record_incoming_from(
        "claimed-first",
        "key-first",
        node,
        "direct",
        SessionDenial::NotGranted,
        false,
    );
    record_incoming_from(
        "claimed-second",
        "key-second",
        node,
        "direct",
        SessionDenial::NotGranted,
        false,
    );
    let mine: Vec<_> = incoming()
        .into_iter()
        .filter(|row| row.node_id == node)
        .collect();
    assert_eq!(mine.len(), 1, "one row for the node");
    assert_eq!(mine[0].count, 2);
    assert_eq!(mine[0].device_id, "claimed-second");
    assert_eq!(mine[0].public_key.as_deref(), Some("key-second"));

    // Another relation kind of the same node is its own row.
    record_incoming_from(
        "claimed-second",
        "key-second",
        node,
        "room",
        SessionDenial::RoomUnknown,
        false,
    );
    assert_eq!(count_with_prefix(node), 2);
}

#[test]
fn entdeckung_task_unpinned_rows_never_displace_pinned_rows_and_hold_at_most_16_places() {
    let _book = CleanBook::new();

    // A book full of pinned rows takes no unpinned row; a new pinned row
    // gives way only to the oldest pinned one.
    for index in 0..MAX_ENTRIES {
        record(
            &format!("pinned-device-{index}"),
            &format!("g15b-pinned-{index}"),
            SessionDenial::NotGranted,
            true,
        );
    }
    for index in 0..20 {
        record(
            &format!("unpinned-device-{index}"),
            &format!("g15b-unpinned-{index}"),
            SessionDenial::NotGranted,
            false,
        );
    }
    assert_eq!(count_with_prefix("g15b-pinned-"), MAX_ENTRIES);
    assert_eq!(count_with_prefix("g15b-unpinned-"), 0);
    record(
        "pinned-device-late",
        "g15b-pinned-late",
        SessionDenial::NotGranted,
        true,
    );
    assert_eq!(incoming().len(), MAX_ENTRIES);
    assert!(incoming()
        .iter()
        .any(|row| row.node_id == "g15b-pinned-late"));
    assert!(incoming().iter().all(|row| row.node_id != "g15b-pinned-0"));

    // Unpinned rows hold at most MAX_UNPINNED places and give way among
    // themselves, oldest first; the pinned rows stay.
    clear_all_incoming();
    for index in 0..10 {
        record(
            &format!("pinned-device-{index}"),
            &format!("g15b-pinned-{index}"),
            SessionDenial::NotGranted,
            true,
        );
    }
    for index in 0..20 {
        record(
            &format!("unpinned-device-{index}"),
            &format!("g15b-unpinned-{index}"),
            SessionDenial::NotGranted,
            false,
        );
    }
    assert_eq!(count_with_prefix("g15b-pinned-"), 10);
    assert_eq!(count_with_prefix("g15b-unpinned-"), MAX_UNPINNED);
    let unpinned: Vec<_> = incoming()
        .into_iter()
        .filter(|row| row.node_id.starts_with("g15b-unpinned-"))
        .map(|row| row.node_id)
        .collect();
    assert!(unpinned.contains(&"g15b-unpinned-19".to_string()));
    assert!(!unpinned.contains(&"g15b-unpinned-0".to_string()));

    // After 64 unpinned attempts a pinned rejection is still recorded and shown.
    clear_all_incoming();
    for index in 0..MAX_ENTRIES {
        record(
            &format!("unpinned-device-{index}"),
            &format!("g15b-unpinned-{index}"),
            SessionDenial::NotGranted,
            false,
        );
    }
    assert_eq!(count_with_prefix("g15b-unpinned-"), MAX_UNPINNED);
    record("known-device", "g15b-known", SessionDenial::Reconfirm, true);
    let known = incoming()
        .into_iter()
        .find(|row| row.node_id == "g15b-known")
        .expect("the pinned rejection is visible");
    assert_eq!(known.denial, Some(SessionDenial::Reconfirm));
}
