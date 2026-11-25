use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use super::*;

fn request(message_type: u16, transaction: [u8; 12], attributes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&message_type.to_be_bytes());
    out.extend_from_slice(&(attributes.len() as u16).to_be_bytes());
    out.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    out.extend_from_slice(&transaction);
    out.extend_from_slice(attributes);
    out
}

/// The address of a Binding success response, decoded as a STUN client
/// does (RFC 8489 14.2), after checking type, length, cookie and ID.
fn mapped(response: &[u8], transaction: [u8; 12]) -> SocketAddr {
    assert_eq!(
        u16::from_be_bytes([response[0], response[1]]),
        BINDING_SUCCESS
    );
    let length = usize::from(u16::from_be_bytes([response[2], response[3]]));
    assert_eq!(response.len(), HEADER_BYTES + length);
    assert_eq!(&response[4..8], &MAGIC_COOKIE.to_be_bytes());
    assert_eq!(&response[8..20], &transaction);
    let attribute = &response[HEADER_BYTES..];
    assert_eq!(
        u16::from_be_bytes([attribute[0], attribute[1]]),
        XOR_MAPPED_ADDRESS
    );
    let value = &attribute[4..4 + usize::from(u16::from_be_bytes([attribute[2], attribute[3]]))];
    let port = u16::from_be_bytes([value[2], value[3]]) ^ 0x2112;
    let mut key = MAGIC_COOKIE.to_be_bytes().to_vec();
    key.extend_from_slice(&transaction);
    let ip: std::net::IpAddr = match value[1] {
        0x01 => {
            let bytes: Vec<u8> = value[4..8].iter().zip(&key).map(|(a, k)| a ^ k).collect();
            std::net::Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]).into()
        }
        0x02 => {
            let mut octets = [0u8; 16];
            for (index, octet) in octets.iter_mut().enumerate() {
                *octet = value[4 + index] ^ key[index];
            }
            std::net::Ipv6Addr::from(octets).into()
        }
        family => panic!("unknown family {family}"),
    };
    SocketAddr::new(ip, port)
}

/// User 2026-11-25 („auch DIREKT UDP“): a Binding request gets the
/// XOR-MAPPED-ADDRESS of its source (IPv4, IPv6, an IPv4-mapped source as
/// IPv4) with its transaction ID, its own attributes ignored; anything else
/// gets no answer.
#[test]
fn stun_task_binding_answers_the_xor_mapped_source_and_ignores_the_rest() {
    let transaction = [7u8, 1, 2, 3, 4, 5, 6, 8, 9, 10, 11, 12];
    let rows = [
        ("IPv4", "203.0.113.7:40000", "203.0.113.7:40000"),
        ("IPv6", "[2001:db8::9]:40001", "[2001:db8::9]:40001"),
        ("mapped", "[::ffff:198.51.100.2]:5", "198.51.100.2:5"),
    ];
    for (case, source, expected) in rows {
        let response = answer(
            &request(BINDING_REQUEST, transaction, &[]),
            source.parse().unwrap(),
        )
        .unwrap_or_else(|| panic!("{case}: no answer"));
        let expected: SocketAddr = expected.parse().unwrap();
        assert_eq!(mapped(&response, transaction), expected, "{case}");
    }
    // A SOFTWARE attribute (0x8022) in the request changes nothing.
    let software = [0x80, 0x22, 0, 4, b's', b'e', b'-', b'1'];
    let response = answer(
        &request(BINDING_REQUEST, transaction, &software),
        "203.0.113.7:1".parse().unwrap(),
    )
    .unwrap();
    let expected: SocketAddr = "203.0.113.7:1".parse().unwrap();
    assert_eq!(mapped(&response, transaction), expected);

    let source: SocketAddr = "203.0.113.7:40000".parse().unwrap();
    let mut wrong_cookie = request(BINDING_REQUEST, transaction, &[]);
    wrong_cookie[4] ^= 1;
    let mut longer = request(BINDING_REQUEST, transaction, &[]);
    longer.extend_from_slice(&[0, 0, 0, 0]);
    let mut top_bits = request(BINDING_REQUEST, transaction, &[]);
    top_bits[0] |= 0x40;
    let ignored = [
        ("indication", request(0x0011, transaction, &[]), source),
        (
            "success response",
            request(BINDING_SUCCESS, transaction, &[]),
            source,
        ),
        ("wrong cookie", wrong_cookie, source),
        ("length mismatch", longer, source),
        ("not a STUN head", top_bits, source),
        (
            "unaligned",
            request(BINDING_REQUEST, transaction, &[1, 2]),
            source,
        ),
        ("short", vec![0, 1, 0, 0], source),
        (
            "port 0",
            request(BINDING_REQUEST, transaction, &[]),
            "203.0.113.7:0".parse().unwrap(),
        ),
    ];
    for (case, datagram, from) in ignored {
        assert!(answer(&datagram, from).is_none(), "{case}");
    }
    assert_eq!(CAPABILITY, "stun_v1");
    assert_eq!(DEFAULT_PORT, 3478);
}

/// Answers go out over the socket only while the admission lets the source
/// through; a datagram without an answer never asks the admission.
#[test]
fn stun_task_admission_bounds_the_answers() {
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    let client = UdpSocket::bind("127.0.0.1:0").unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    client
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    let server_address = server.local_addr().unwrap();
    let mut asked = 0;
    let mut admit = |_: SocketAddr| {
        asked += 1;
        asked <= 2
    };
    let mut buffer = [0u8; 1500];
    let mut sent = Vec::new();
    for round in 0u8..3 {
        let transaction = [round; 12];
        client
            .send_to(&request(BINDING_REQUEST, transaction, &[]), server_address)
            .unwrap();
        let (length, source) = server.recv_from(&mut buffer).unwrap();
        sent.push(handle(&server, &buffer[..length], source, &mut admit));
    }
    client.send_to(&[0u8; 7], server_address).unwrap();
    let (length, source) = server.recv_from(&mut buffer).unwrap();
    assert!(!handle(&server, &buffer[..length], source, &mut admit));
    assert_eq!(sent, [true, true, false]);
    assert_eq!(asked, 3, "the garbage datagram did not ask");

    for round in 0u8..2 {
        let (length, _) = client.recv_from(&mut buffer).unwrap();
        assert_eq!(
            mapped(&buffer[..length], [round; 12]),
            client.local_addr().unwrap()
        );
    }
    assert!(client.recv_from(&mut buffer).is_err(), "no third answer");
}
