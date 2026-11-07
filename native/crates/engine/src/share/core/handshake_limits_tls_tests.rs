//! Slots of the TLS handshakes before any identity is known (R-ZUGANG-01):
//! a quarter of the slots per source, a QUIC Retry for unvalidated direct
//! senders from half of the slots, a quarter reserved for pinned relay
//! senders, and the grouping of the sources.
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use super::*;

fn v4(last: u8) -> TlsSource {
    TlsSource::of_ip(IpAddr::V4(Ipv4Addr::new(203, 0, 113, last)))
}

fn relay(name: &str) -> TlsSource {
    TlsSource::Relay(name.to_string())
}

/// `count` slots of `source`; any other decision fails the test.
fn take(
    slots: &Arc<TlsHandshakeSlots>,
    source: &TlsSource,
    validated: bool,
    pinned: bool,
    count: usize,
) -> Vec<TlsSlot> {
    (0..count)
        .map(
            |index| match slots.admit(source.clone(), validated, pinned) {
                TlsAdmission::Slot(slot) => slot,
                TlsAdmission::Retry => panic!("Retry instead of slot {index} of {source:?}"),
                TlsAdmission::Refuse => panic!("Refuse instead of slot {index} of {source:?}"),
            },
        )
        .collect()
}

fn refused(
    slots: &Arc<TlsHandshakeSlots>,
    source: &TlsSource,
    validated: bool,
    pinned: bool,
) -> bool {
    matches!(
        slots.admit(source.clone(), validated, pinned),
        TlsAdmission::Refuse
    )
}

fn retried(
    slots: &Arc<TlsHandshakeSlots>,
    source: &TlsSource,
    validated: bool,
    pinned: bool,
) -> bool {
    matches!(
        slots.admit(source.clone(), validated, pinned),
        TlsAdmission::Retry
    )
}

#[test]
fn zugang_task_tls_slots_hold_a_quarter_per_source_and_free_on_drop() {
    let slots = TlsHandshakeSlots::new(64);
    let (a, b) = (v4(1), v4(2));
    let mut held = take(&slots, &a, true, false, 16);
    assert!(
        refused(&slots, &a, true, false),
        "the 17th slot of one source"
    );
    assert_eq!(slots.held(&a), (16, 16));
    // Another source is not affected by the full one.
    let b_slot = take(&slots, &b, true, false, 1);
    assert_eq!(slots.held(&b), (17, 1));

    // Dropping one slot frees the total and the source count by one.
    drop(held.pop());
    assert_eq!(slots.held(&a), (16, 15));
    held.extend(take(&slots, &a, true, false, 1));
    assert!(refused(&slots, &a, true, false));

    // A source leaves the table at zero; the host can take 16 of it again.
    drop(held);
    assert_eq!(slots.held(&a), (1, 0));
    assert!(!slots.lock().sources.contains_key(&a));
    drop(b_slot);
    assert_eq!(slots.held(&b), (0, 0));
    assert!(slots.lock().sources.is_empty());
    let again = take(&slots, &a, true, false, 16);
    assert_eq!(slots.held(&a), (16, 16));
    drop(again);
}

#[test]
fn zugang_task_tls_slots_retry_unvalidated_direct_senders_under_load() {
    let slots = TlsHandshakeSlots::new(64);
    let (a, b, c) = (v4(1), v4(2), v4(3));
    let mut keep = take(&slots, &a, true, false, 16);
    keep.extend(take(&slots, &b, true, false, 16));
    assert_eq!(slots.held(&c), (32, 0));

    // From half of the slots an unvalidated direct sender gets a Retry and
    // no slot; the same sender with a validated address gets a slot.
    assert!(retried(&slots, &c, false, false));
    assert!(retried(&slots, &TlsSource::V6(7), false, false));
    assert_eq!(slots.held(&c), (32, 0), "a Retry takes no slot");
    keep.extend(take(&slots, &c, true, false, 1));
    assert_eq!(slots.held(&c), (33, 1));

    // Relay and other transports cannot forge an address: no Retry.
    let x = relay("relay-x");
    keep.extend(take(&slots, &x, false, false, 1));
    assert_eq!(slots.held(&x), (34, 1));
    keep.extend(take(
        &slots,
        &TlsSource::Other("custom".into()),
        false,
        false,
        1,
    ));
    assert_eq!(slots.held(&x), (35, 1));
    drop(keep);

    // The boundary is the 32nd taken slot: below it nobody gets a Retry.
    let slots = TlsHandshakeSlots::new(64);
    let mut keep = take(&slots, &a, false, false, 16);
    keep.extend(take(&slots, &b, false, false, 15));
    assert_eq!(slots.held(&c), (31, 0));
    keep.extend(take(&slots, &c, false, false, 1));
    assert_eq!(slots.held(&c), (32, 1));
    assert!(retried(&slots, &c, false, false));
    assert_eq!(slots.held(&c), (32, 1));
    drop(keep);
}

#[test]
fn zugang_task_tls_slots_reserve_a_quarter_for_pinned_relay_senders() {
    let slots = TlsHandshakeSlots::new(64);
    let mut keep = Vec::new();
    for last in 1..=3 {
        keep.extend(take(&slots, &v4(last), true, false, 16));
    }
    assert_eq!(slots.held(&v4(9)), (48, 0));

    // The last 16 slots are not for direct senders, not for relay senders
    // this host does not pin, and not for other transports.
    assert!(refused(&slots, &v4(4), true, false), "direct sender");
    assert!(
        refused(&slots, &v4(5), true, true),
        "pinned flag of a direct sender"
    );
    assert!(
        refused(&slots, &relay("relay-y"), false, false),
        "unpinned relay"
    );
    assert!(refused(
        &slots,
        &TlsSource::Other("custom".into()),
        false,
        true
    ));
    assert_eq!(slots.held(&v4(9)), (48, 0));

    // A relay sender this host pins takes the reserve, up to its quarter.
    let z = relay("relay-z");
    keep.extend(take(&slots, &z, false, true, 16));
    assert_eq!(slots.held(&z), (64, 16));
    assert!(refused(&slots, &z, false, true), "17th slot of one source");
    assert!(
        refused(&slots, &relay("relay-z2"), false, true),
        "all 64 slots taken"
    );
    drop(keep);
    assert_eq!(slots.held(&z), (0, 0));
}

#[test]
fn zugang_task_tls_sources_group_mapped_v4_and_v6_prefixes() {
    let ip = |text: &str| TlsSource::of_ip(text.parse::<IpAddr>().expect("test address"));
    // IPv4-mapped IPv6 is the IPv4 address.
    assert_eq!(ip("::ffff:203.0.113.9"), ip("203.0.113.9"));
    assert_eq!(ip("203.0.113.9"), v4(9));
    assert_ne!(ip("203.0.113.9"), ip("203.0.113.10"));
    // One /64 prefix is one source; another prefix is another one.
    assert_eq!(ip("2001:db8:1:2::1"), ip("2001:db8:1:2::ffff"));
    assert_ne!(ip("2001:db8:1:2::1"), ip("2001:db8:1:3::1"));
    assert!(matches!(ip("2001:db8:1:2::1"), TlsSource::V6(_)));

    // The source of an incoming connection follows its address.
    let socket = |text: &str| SocketAddr::new(text.parse().expect("test address"), 4433);
    assert_eq!(
        TlsSource::of(&IncomingAddr::Ip(socket("203.0.113.9"))),
        ip("203.0.113.9")
    );
    assert_eq!(
        TlsSource::of(&IncomingAddr::Ip(socket("2001:db8:1:2::7"))),
        ip("2001:db8:1:2::1")
    );
    let endpoint_id = iroh::SecretKey::from_bytes(&[41; 32]).public();
    let url: iroh::RelayUrl = "https://relay.example./".parse().expect("relay url");
    assert_eq!(
        TlsSource::of(&IncomingAddr::Relay { url, endpoint_id }),
        relay(&endpoint_id.to_string())
    );
}
