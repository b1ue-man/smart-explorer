use super::*;
use crate::screen::core::video_transport::udp_wire::Side;

const KEY: [u8; 32] = [3; 32];

fn addr(text: &str) -> SocketAddr {
    text.parse().unwrap()
}

fn pair() -> (Checks, Checks, SocketAddr, SocketAddr) {
    let host_addr = addr("198.51.100.1:4000");
    let viewer_addr = addr("203.0.113.9:5000");
    let unreachable = addr("10.9.9.9:5000");
    let host = Checks::new(
        Arc::new(Envelope::new(&KEY, 5, Side::Host).unwrap()),
        vec![unreachable, viewer_addr],
        100,
        0,
    );
    let viewer = Checks::new(
        Arc::new(Envelope::new(&KEY, 5, Side::Viewer).unwrap()),
        vec![host_addr],
        200,
        0,
    );
    (host, viewer, host_addr, viewer_addr)
}

#[test]
fn udp_task_checks_validate_the_answered_pair_only() {
    let (mut host, mut viewer, host_addr, viewer_addr) = pair();
    let sent = host.tick(0);
    assert_eq!(sent.len(), 2, "one check per candidate");
    assert!(host.tick(1).is_empty(), "the next round waits");
    let (check, to) = sent
        .iter()
        .find(|(_, to)| *to == viewer_addr)
        .cloned()
        .unwrap();
    assert_eq!(to, viewer_addr);
    // The viewer answers to where the check came from.
    let Incoming::Reply { packet: answer, to } = viewer.on_packet(&check, host_addr, 10) else {
        panic!("a check is answered");
    };
    assert_eq!(to, host_addr);
    // The same answer from another address (replayed there) validates
    // nothing; from the checked address it does.
    host.on_packet(&answer, addr("192.0.2.66:7000"), 20);
    assert_eq!(host.selected(), None);
    host.on_packet(&answer, viewer_addr, 20);
    assert_eq!(host.selected(), Some(viewer_addr));
    assert_eq!(host.take_change(), Some(Some(viewer_addr)));
    assert_eq!(host.take_change(), None);
    // From now on only the pair is checked, every half second.
    assert!(host.tick(21).is_empty());
    let keepalive = host.tick(KEEPALIVE_US + 20);
    assert_eq!(keepalive.len(), 1);
    assert_eq!(keepalive[0].1, viewer_addr);
    // A picture packet of the host opens on the viewer; the viewer's own
    // checks learn its NAT mapping on the host (peer-reflexive).
    let picture = host.envelope().seal(UdpType::Picture, b"shard").unwrap();
    assert_eq!(
        viewer.on_packet(&picture, host_addr, 30),
        Incoming::Picture(b"shard".to_vec())
    );
    let (check, _) = viewer.tick(0).remove(0);
    let mapped = addr("203.0.113.9:6000");
    assert!(matches!(
        host.on_packet(&check, mapped, 40),
        Incoming::Reply { to, .. } if to == mapped
    ));
    // A packet of another session is nothing.
    let stranger = Envelope::new(&KEY, 6, Side::Host).unwrap();
    let foreign = stranger.seal(UdpType::Picture, b"x").unwrap();
    assert_eq!(viewer.on_packet(&foreign, host_addr, 50), Incoming::Nothing);
}

#[test]
fn udp_task_checks_drop_a_pair_without_consent_and_start_over() {
    let (mut host, mut viewer, host_addr, viewer_addr) = pair();
    let (check, _) = host
        .tick(0)
        .into_iter()
        .find(|(_, to)| *to == viewer_addr)
        .unwrap();
    let Incoming::Reply { packet, .. } = viewer.on_packet(&check, host_addr, 0) else {
        panic!("a check is answered");
    };
    host.on_packet(&packet, viewer_addr, 0);
    host.take_change();
    // Keepalives answered keep the pair.
    let (check, _) = host.tick(KEEPALIVE_US).remove(0);
    let Incoming::Reply { packet, .. } = viewer.on_packet(&check, host_addr, KEEPALIVE_US) else {
        panic!("a keepalive is answered");
    };
    host.on_packet(&packet, viewer_addr, KEEPALIVE_US);
    host.tick(CONSENT_US);
    assert_eq!(host.selected(), Some(viewer_addr));
    // Silent for the consent time: dropped, and every candidate is
    // checked again at the fast pace.
    let silent = KEEPALIVE_US + CONSENT_US;
    assert_eq!(host.due_us(), silent);
    let again = host.tick(silent);
    assert_eq!(host.selected(), None);
    assert_eq!(host.take_change(), Some(None));
    assert_eq!(again.len(), 2);
    assert!(host.tick(silent + CHECK_FAST_US - 1).is_empty());
    assert_eq!(host.tick(silent + CHECK_FAST_US).len(), 2);
    // After the fast phase the checks slow down.
    let slow = silent + FAST_FOR_US;
    assert_eq!(host.tick(slow).len(), 2);
    assert!(host.tick(slow + CHECK_FAST_US).is_empty());
    assert_eq!(host.tick(slow + CHECK_SLOW_US).len(), 2);
}
