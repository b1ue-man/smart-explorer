use super::*;
use super::super::lan_link_wire::{self, LinkFrame};

fn identity(seed: u8, device: &str) -> DirectPeerIdentity {
    DirectPeerIdentity::from_secret(device, "", &iroh::SecretKey::from_bytes(&[seed; 32]))
}

fn contact(peer: &DirectPeerIdentity) -> DirectContact {
    DirectContact {
        id: "saved-peer".into(), display_name: "Peer".into(), lookup_id: "lookup-peer".into(),
        expected_fingerprint: peer.fingerprint.clone(), expected_node_id: peer.node_id.clone(),
        remote_device_id: Some(peer.device_id.clone()), remote_public_key: Some(peer.public_key.clone()),
        accepted_public_key: Some(peer.public_key.clone()), auto_connect: false, auto_open: false,
        last_seen: None, status: Default::default(), last_error: None, presence: None,
        access_state: DirectAccessState::Accepted, request_sent_at: None, accepted_at: Some(1),
        lan_candidates: Vec::new(), lan_seen_at: None, lan_uplink: None, relation: Default::default(),
    }
}

fn iface(index: u32, ip: &str) -> InterfaceFacts {
    InterfaceFacts { name: format!("if{index}"), adapter_id: format!("adapter-{index}"), index,
        up: true, loopback: false, addrs: vec![ip.parse().unwrap()], has_gateway: false, dhcp_lease: Some(false) }
}

fn fact(pin: LanPeerPin, facts: &[InterfaceFacts]) -> AuthenticatedLanFact {
    let remote = "169.254.1.2:4242".parse().unwrap();
    let local = "169.254.1.1".parse().unwrap();
    AuthenticatedLanFact { pin, interface: private_interface(local, remote, facts).unwrap(),
        remote_addr: remote, path_id: "current-path".into(), connection_id: 1, challenge: [9; 32],
        peer_uplink: false, confirmed_at: 100, expires_at: 108 }
}

#[test]
fn review_task_s09_full_current_pins_are_required_for_channel_and_worker() {
    let peer = identity(1, "device-a");
    let mut saved = contact(&peer);
    let pin = contact_pin(&saved).unwrap();
    assert!(pin.matches_tls_identity(&peer, &peer.node_id));
    assert!(pin_current(&pin, &[saved.clone()], &[]));
    let wrong = identity(2, "device-a");
    assert!(!pin.matches_tls_identity(&wrong, &wrong.node_id));
    let mut alias = peer.clone();
    alias.device_id = "other-device".into();
    assert!(!pin.matches_tls_identity(&alias, &peer.node_id));
    saved.expected_fingerprint = "changed".into();
    assert!(!pin_current(&pin, &[saved.clone()], &[]));
    saved = contact(&peer);
    saved.access_state = DirectAccessState::Ignored;
    assert!(!pin_current(&pin, &[saved], &[]));
    let mut grant = DirectGrant { device_id: peer.device_id.clone(), device_name: String::new(),
        public_key: peer.public_key.clone(), fingerprint: peer.fingerprint.clone(), node_id: peer.node_id.clone(),
        state: DirectGrantState::Accepted, updated_at: 1, exec: Default::default(), write: false };
    let granted = grant_pin(&grant).unwrap();
    assert!(granted.matches_tls_identity(&peer, &peer.node_id));
    grant.state = DirectGrantState::Reconfirm;
    assert!(!pin_current(&granted, &[], &[grant]));
}

#[test]
fn review_task_s09_exact_ip_rejects_ambiguous_interfaces_and_unknown_dhcp() {
    let local = "169.254.1.1".parse().unwrap();
    let remote = "169.254.1.2:4242".parse().unwrap();
    let facts = vec![iface(2, "169.254.1.1"), iface(3, "169.254.1.9")];
    assert_eq!(private_interface(local, remote, &facts).unwrap().index, 2);
    let mut duplicate = facts.clone();
    duplicate[1].addrs.push(local);
    assert!(private_interface(local, remote, &duplicate).is_none());
    assert!(private_interface("169.254.1.8".parse().unwrap(), remote, &facts).is_none());
    assert!(private_interface(local, "203.0.113.2:4242".parse().unwrap(), &facts).is_none());
    let peer = identity(1, "device-a");
    let checked = fact(contact_pin(&contact(&peer)).unwrap(), &facts);
    assert!(checked.can_share_on(&facts, &[]));
    let mut unknown = facts.clone();
    unknown[0].dhcp_lease = None;
    assert!(!checked.can_share_on(&unknown, &[]));
    let mut receiving = facts;
    receiving[0].has_gateway = true;
    receiving[0].dhcp_lease = Some(true);
    // Status remains usable after receiving DHCP from the sharing peer;
    // that routed interface does not acquire new NAT/DHCP authority.
    assert!(private_interface(local, remote, &receiving).is_some());
    assert!(!checked.can_share_on(&receiving, &[]));
}

#[test]
fn review_task_s09_ipv6_uses_exact_local_ip_and_rejects_wrong_scope() {
    let facts = vec![iface(7, "fe80::1"), iface(8, "fe80::2")];
    let local = "fe80::1".parse().unwrap();
    let right = crate::net::parse_candidate("[fe80::9%7]:4242").unwrap();
    let wrong = crate::net::parse_candidate("[fe80::9%8]:4242").unwrap();
    assert_eq!(private_interface(local, right, &facts).unwrap().index, 7);
    assert!(private_interface(local, wrong, &facts).is_none());
}

#[test]
fn review_task_s09_old_facts_and_changed_adapters_cannot_authorize() {
    let peer = identity(1, "device-a");
    let contacts = [contact(&peer)];
    let facts = [iface(2, "169.254.1.1")];
    let mut checked = fact(contact_pin(&contacts[0]).unwrap(), &facts);
    assert!(checked.current(107, &contacts, &[], &facts));
    assert!(!checked.current(99, &contacts, &[], &facts));
    assert!(!checked.current(108, &contacts, &[], &facts));
    let mut changed = facts.clone();
    changed[0].adapter_id = "replacement-adapter".into();
    assert!(!checked.current(107, &contacts, &[], &changed));
    checked.expires_at = 1000;
    assert!(!checked.current(107, &contacts, &[], &facts));
    checked.expires_at = 108;
    checked.challenge.fill(0);
    assert!(!checked.current(107, &contacts, &[], &facts));
}

#[test]
fn review_task_s09_missing_net_verdict_is_unknown_not_no_uplink() {
    let facts = [iface(2, "169.254.1.1")];
    assert_eq!(OwnUplink::from_interfaces(&facts, None, &[], &[]), OwnUplink::Unknown);
    assert_eq!(OwnUplink::from_interfaces(&[], Some(&[]), &[], &[]), OwnUplink::Unknown);
    assert_eq!(OwnUplink::from_interfaces(&facts, Some(&[3]), &[], &[]), OwnUplink::Unknown);
    assert_eq!(OwnUplink::from_interfaces(&facts, Some(&[2]), &[], &[]), OwnUplink::Present);
    assert_eq!(OwnUplink::from_interfaces(&facts, Some(&[2]), &[2], &[]), OwnUplink::Absent);
    assert_eq!(OwnUplink::from_interfaces(&facts, Some(&[]), &[], &[]), OwnUplink::Absent);
}

#[test]
fn review_task_s09_challenge_replay_and_non_status_frames_are_rejected() {
    let first = super::super::core::random_bytes::<32>().unwrap();
    let next = super::super::core::random_bytes::<32>().unwrap();
    assert!(lan_link_wire::check_echo(&first, &first).is_ok());
    assert!(lan_link_wire::check_echo(&next, &first).is_err());
    let frame = LinkFrame::Answer { echo: first, nonce: next, identity: identity(1, "device-a"), uplink: OwnUplink::Unknown };
    let bytes = lan_link_wire::encode(&frame).unwrap();
    match lan_link_wire::decode(&bytes).unwrap() {
        LinkFrame::Answer { uplink, .. } => assert_eq!(uplink.known(), None),
        _ => panic!("expected status answer"),
    }
    assert!(lan_link_wire::decode(br#"{"type":"exec","argv":["touch","private"]}"#).is_err());
    assert!(lan_link_wire::decode(&vec![b' '; lan_link_wire::MAX_LINK_FRAME + 1]).is_err());
    assert!(lan_link_wire::decode(&[]).is_err());
}
