use super::*;

const KEY: [u8; 32] = [7; 32];

#[test]
fn udp_task_pair_packets_are_sealed_for_one_session_and_side() {
    let host = Envelope::new(&KEY, 99, Side::Host).unwrap();
    let viewer = Envelope::new(&KEY, 99, Side::Viewer).unwrap();
    let inner = b"picture packet".to_vec();
    let first = host.seal(UdpType::Picture, &inner).unwrap();
    let second = host.seal(UdpType::Picture, &inner).unwrap();
    assert_eq!(first.len(), ENVELOPE_HEAD + inner.len() + TAG_BYTES);
    assert_eq!(first[0], UDP_KIND);
    // Every packet has its own nonce: the same bytes seal differently.
    assert_ne!(first, second);
    assert_eq!(viewer.open(&first), Some((UdpType::Picture, inner.clone())));
    assert_eq!(
        viewer.open(&second),
        Some((UdpType::Picture, inner.clone()))
    );
    // A side never opens its own packets (the direction is in the nonce).
    assert_eq!(host.open(&first), None);
    // A changed byte of head or payload, another session or key: refused.
    for index in [1, 5, 12, ENVELOPE_HEAD, first.len() - 1] {
        let mut changed = first.clone();
        changed[index] ^= 1;
        assert_eq!(viewer.open(&changed), None, "byte {index}");
    }
    let other_session = Envelope::new(&KEY, 98, Side::Viewer).unwrap();
    assert_eq!(other_session.open(&first), None);
    let other_key = Envelope::new(&[8; 32], 99, Side::Viewer).unwrap();
    assert_eq!(other_key.open(&first), None);
    // Checks go the other way; the inner bytes have their limit.
    let check = viewer.seal(UdpType::Check, &5u64.to_be_bytes()).unwrap();
    assert_eq!(
        host.open(&check),
        Some((UdpType::Check, 5u64.to_be_bytes().to_vec()))
    );
    assert!(host.seal(UdpType::Picture, &vec![0; MAX_INNER]).is_ok());
    assert!(host
        .seal(UdpType::Picture, &vec![0; MAX_INNER + 1])
        .is_err());
    assert_eq!(MAX_INNER + ENVELOPE_HEAD + TAG_BYTES, MAX_UDP_PAYLOAD);
    // A pair packet is no STUN message.
    assert!(!is_stun(&first));
}

/// A Binding success response with one attribute.
fn response(transaction: &[u8; 12], kind: u16, value: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0x0101u16.to_be_bytes());
    let padded = value.len().div_ceil(4) * 4;
    out.extend_from_slice(&((4 + padded) as u16).to_be_bytes());
    out.extend_from_slice(&STUN_MAGIC.to_be_bytes());
    out.extend_from_slice(transaction);
    out.extend_from_slice(&kind.to_be_bytes());
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value);
    out.resize(out.len() + padded - value.len(), 0);
    out
}

#[test]
fn udp_task_stun_request_and_the_mappings_of_rfc_5769() {
    // RFC 5769 2.2 and 2.3: transaction id and XOR-MAPPED-ADDRESS values.
    let transaction = [
        0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae,
    ];
    let request = stun_request(&transaction);
    assert_eq!(&request[..8], &[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
    assert_eq!(&request[8..], &transaction);
    assert!(is_stun(&request));
    let v4 = [0x00, 0x01, 0xa1, 0x47, 0xe1, 0x12, 0xa6, 0x43];
    let mapped = stun_mapping(&response(&transaction, 0x0020, &v4), &transaction);
    assert_eq!(mapped, Some("192.0.2.1:32853".parse().unwrap()));
    let v6 = [
        0x00, 0x02, 0xa1, 0x47, 0x01, 0x13, 0xa9, 0xfa, 0xa5, 0xd3, 0xf1, 0x79, 0xbc, 0x25, 0xf4,
        0xb5, 0xbe, 0xd2, 0xb9, 0xd9,
    ];
    let mapped = stun_mapping(&response(&transaction, 0x0020, &v6), &transaction);
    assert_eq!(
        mapped,
        Some(
            "[2001:db8:1234:5678:11:2233:4455:6677]:32853"
                .parse()
                .unwrap()
        )
    );
    // An old server's MAPPED-ADDRESS, plain.
    let plain = [0x00, 0x01, 0x80, 0x55, 192, 0, 2, 1];
    let mapped = stun_mapping(&response(&transaction, 0x0001, &plain), &transaction);
    assert_eq!(mapped, Some("192.0.2.1:32853".parse().unwrap()));
    // Another transaction, a request or a cut message: no mapping.
    let mut other = transaction;
    other[0] ^= 1;
    assert_eq!(
        stun_mapping(&response(&transaction, 0x0020, &v4), &other),
        None
    );
    assert_eq!(stun_mapping(&request, &transaction), None);
    let cut = response(&transaction, 0x0020, &v4);
    assert_eq!(stun_mapping(&cut[..cut.len() - 1], &transaction), None);
}
