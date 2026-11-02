use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};

use super::{certificate_matches_pin, handshake, relay_ca_tls_config, websocket_config};
use crate::share::core::hex;
use crate::share::line::MAX_SIGNAL_LINE;
use crate::share::server_address::SignalServerConfig;

// Leaf certificate (SAN localhost, 127.0.0.1, valid until 2126) and its key,
// the same test material as the FTPS tests.
const TEST_LEAF_DER: &str = "MIIDXDCCAkSgAwIBAgIUWHuMJcN38PTITCkCtvIQZhkeooUwDQYJKoZIhvcNAQELBQAwJTEjMCEGA1UEAwwaU21hcnQgRXhwbG9yZXIgRlRQIFRlc3QgQ0EwIBcNMjYwNzEyMTgyMTI2WhgPMjEyNjA2MTgxODIxMjZaMBQxEjAQBgNVBAMMCWxvY2FsaG9zdDCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAM1dcpTzeAWLN2xIwfoPNLYyf8Eg5+/AX8T58dpgonZq4fYiP92YXbZYe/IFYSeHpxSdq6byR0TMb0+qpotacalypwXlazZoIMG6rO8KgRMVAsTLRIf22DKh9MXHVgr/tqKM5WXJmRxewvQRtRIs2+J8lhZwbC0HnNY/vO5WBC2PsI1V3m6kXrq0cYC292KZTWPxdHr8uek+sYjezxVuGVmTt8eLaqZUfVJ2UeR6AZ+NtqoW7vTz05C/Eb012BAS+bzacMCiBXwimpdwXRUvAjwGsRAQ3qHhgpffff2OQdJM/+tLB2L2wUzJBIVruDaTOKZtGwjbfkhVv6czKLElzRMCAwEAAaOBkjCBjzAMBgNVHRMBAf8EAjAAMA4GA1UdDwEB/wQEAwIFoDATBgNVHSUEDDAKBggrBgEFBQcDATAaBgNVHREEEzARgglsb2NhbGhvc3SHBH8AAAEwHQYDVR0OBBYEFIxN+EcoYOvoTZ330WrYUFI8U62xMB8GA1UdIwQYMBaAFDZkKjrgRWBSTP5M5veFf3phLei5MA0GCSqGSIb3DQEBCwUAA4IBAQBweKOh3cF412BfQ9CVowdVpNIsnjRTBztnfQYwgwV4zGcFYfSKupf14tIPnIK9F+Jef8CpqPS878KUspd4u1dN+oYfiIB4BPyZH7GkYey69jQ4+tSQCVeDEn7dGNOMDvDelaNFU5DUTLjEiccuEY7MvlaePtyckb/ipBf4eLHYJ5lD3XQt2fvcYTZgIbvSdMMBTwzV3HWA6m9jee2Tj5Zd8q108amm7bBw235Fk3uZeN6+SQ65MfEAlbXsf893zweJO8JVfqY3NO+ucd/9gyNoTg3TVYu/fC0UI2QXlAFAiZ7p33hpXBdbw3qYhufgrEqeT2TNoJIO/78xydnEYlNt";
const TEST_LEAF_KEY_DER: &str = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDNXXKU83gFizdsSMH6DzS2Mn/BIOfvwF/E+fHaYKJ2auH2Ij/dmF22WHvyBWEnh6cUnaum8kdEzG9PqqaLWnGpcqcF5Ws2aCDBuqzvCoETFQLEy0SH9tgyofTFx1YK/7aijOVlyZkcXsL0EbUSLNvifJYWcGwtB5zWP7zuVgQtj7CNVd5upF66tHGAtvdimU1j8XR6/LnpPrGI3s8VbhlZk7fHi2qmVH1SdlHkegGfjbaqFu7089OQvxG9NdgQEvm82nDAogV8IpqXcF0VLwI8BrEQEN6h4YKX3339jkHSTP/rSwdi9sFMyQSFa7g2kzimbRsI235IVb+nMyixJc0TAgMBAAECggEAWMqYAXW5BXCZUGyuzb6oVERGP0rKbTsYTTKiEoCojZmNxB0vztATaIUeZdhUlsJMh5naPw7OqJzZXbETW/oJXbGQLHjyX24rB4f+QEYi44y4izy1jzG3bUDf82lJtuyz2tkfT+CXnhAMq3lCeC7EDUs/m0kVRGzfrzSUq9mt6cJJiJEJb0TOlu7aRlhZfIr5PcPCJ4x0kUPtlUmOb7C1w9BnaVcA7WMkyzCQzg6lIK6iN2PCNY2WRUrjjZKKZTZXesovIBu828clGkcKA5WQpV/EjI/fRMWMkBTbCW3oiqerNORqs0NeGAnpCeVxXy/wR6AfytIB9N9pCHjjQ4jPaQKBgQDyT3xt6dxXbi7laQ9rZbVmaTaGq48v+goTGzR8yUt8dzuEnRJhXuw9J5ZYua9mBbjJCIiFoVVCypxl7O+miXuaoUQCWP7hVj1K428ib0XQk6zlWYzkm4hb+K3B87maFq410c8Xsj7DXPRNvXRfsrvEGUQYshPzqJW8LQyx1b4+qwKBgQDY959tpuX6wJAI1yVzN0f/oyBMRM8ZEU+rehdV4u0JvMN5hnNJrt7w7S3J1EkaqR2uKIPyztJUtDfnY1on3GeCS9330GGU963gR7OCxAScpoR8JZCog2goqZkt/mlUMsjnzy2L9dCTjiPEHQlDcxx63bM1Kwyc8VjnCrM5zUmLOQKBgQDqRNMmaU3w8cRBZJvV19XUF7Dx7vhXCEWpR0otw2hKA/T1N+9HWMDKN3XyfkQIPUv0gV2M5PhLxRwEp1jkCFQKohPguS5jqj9EIjOWdUJob/5fF39SntTtJrbHp94wDfGMczbn0BtCQqKobp0O0P0ckNj3j2Qe1UU/U8bMQLzYVQKBgGwgtBZ8h764us99EU/jLAGNtWntHNzcUL0fooOODR2+MhjdVZVSDg851IjyP+CGiaEi1edrBU1rZzTswaB96iP4VU3MTuVjrgbJFQBFWhsLrZkFS5t/qagiJZHTaYCpspA8IvHOdr0iqFZzNgukUXw2ArqrkqSgbvLt1TYoRc+ZAoGAHDpbCScU12C/18aG3yKA9ApjmbdtA/paJvdCIKWEBaDUYFS4CYdvynrqzZl348jtnjrBdK6zhr+uJLf0ndSoLr/Arj3fXf9Jjp5oO+ryif8OtXu+RYD8pbWa3tkb/9PwQtkrRPYyfSlKju0xoXEG71EUKA5UlsLPFmwGpXVHq90=";

fn decode(value: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .expect("test DER")
}

fn leaf() -> CertificateDer<'static> {
    CertificateDer::from(decode(TEST_LEAF_DER))
}

fn certificate_pin(cert: &CertificateDer<'_>) -> [u8; 32] {
    Sha256::digest(cert.as_ref()).into()
}

/// One TLS WebSocket accept with the test leaf; sends one text message.
fn tls_websocket_server() -> (std::net::SocketAddr, std::thread::JoinHandle<bool>) {
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(decode(TEST_LEAF_KEY_DER)));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_no_client_auth()
        .with_single_cert(vec![leaf()], key)
        .expect("server certificate");
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = std::thread::spawn(move || {
        let Ok((tcp, _)) = listener.accept() else {
            return false;
        };
        let _ = tcp.set_read_timeout(Some(Duration::from_secs(10)));
        let _ = tcp.set_write_timeout(Some(Duration::from_secs(10)));
        let Ok(connection) = rustls::ServerConnection::new(Arc::new(config)) else {
            return false;
        };
        let Ok(mut socket) = tungstenite::accept(rustls::StreamOwned::new(connection, tcp)) else {
            return false;
        };
        socket
            .send(tungstenite::Message::Text(r#"{"t":"pong"}"#.to_string()))
            .is_ok()
    });
    (address, server)
}

fn client_socket(address: std::net::SocketAddr) -> TcpStream {
    let tcp = TcpStream::connect(address).expect("connect");
    tcp.set_read_timeout(Some(Duration::from_secs(10)))
        .expect("read timeout");
    tcp.set_write_timeout(Some(Duration::from_secs(10)))
        .expect("write timeout");
    tcp
}

#[test]
fn review_task_pin_matches_the_certificate_or_its_public_key() {
    let cert = leaf();
    assert!(certificate_matches_pin(&cert, &certificate_pin(&cert)));
    let parsed = rustls::server::ParsedCertificate::try_from(&cert).expect("parsed leaf");
    let spki: [u8; 32] = Sha256::digest(parsed.subject_public_key_info().as_ref()).into();
    assert!(certificate_matches_pin(&cert, &spki));
    assert!(!certificate_matches_pin(&cert, &[7; 32]));
}

#[test]
fn review_task_pinned_wss_server_is_accepted_and_a_wrong_pin_is_refused() {
    let pin = certificate_pin(&leaf());
    let (address, server) = tls_websocket_server();
    let config = SignalServerConfig::parse_input(
        &format!("wss://127.0.0.1:{}#sha256={}", address.port(), hex(&pin)),
        false,
    )
    .expect("pinned address");
    let mut socket = handshake(&config.endpoints()[0], client_socket(address)).expect("pinned TLS");
    let message = socket.read().expect("server message");
    assert_eq!(message.into_text().expect("text"), r#"{"t":"pong"}"#);
    assert!(server.join().expect("server thread"));

    let (address, server) = tls_websocket_server();
    let wrong = SignalServerConfig::parse_input(
        &format!(
            "wss://127.0.0.1:{}#sha256={}",
            address.port(),
            hex(&[9; 32])
        ),
        false,
    )
    .expect("wrong pin address");
    let error = handshake(&wrong.endpoints()[0], client_socket(address)).unwrap_err();
    assert!(error.to_string().contains("abgelehnt"), "{error}");
    assert!(!server.join().expect("server thread"));
}

#[test]
fn review_task_unpinned_self_signed_server_is_refused() {
    let (address, server) = tls_websocket_server();
    let config =
        SignalServerConfig::parse_input(&format!("wss://127.0.0.1:{}", address.port()), false)
            .expect("address");
    assert!(handshake(&config.endpoints()[0], client_socket(address)).is_err());
    assert!(!server.join().expect("server thread"));
}

#[test]
fn review_task_signaling_websocket_is_limited_to_one_signal_line() {
    let config = websocket_config();
    assert_eq!(config.max_message_size, Some(MAX_SIGNAL_LINE));
    assert_eq!(config.max_frame_size, Some(MAX_SIGNAL_LINE));
}

#[test]
fn review_task_relay_trust_changes_only_for_pinned_servers() {
    assert!(relay_ca_tls_config(&[]).is_none());
    assert!(relay_ca_tls_config(&[[1; 32]]).is_some());
}
