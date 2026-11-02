use super::*;

use base64::Engine as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;

use super::super::io_adapters::{FtpConnection, FtpReconnect};

const TEST_CA_DER: &str = "MIIDPTCCAiWgAwIBAgIULL911JraapAUxpLhTEDrkTtg5+8wDQYJKoZIhvcNAQELBQAwJTEjMCEGA1UEAwwaU21hcnQgRXhwbG9yZXIgRlRQIFRlc3QgQ0EwIBcNMjYwNzEyMTgyMTI2WhgPMjEyNjA2MTgxODIxMjZaMCUxIzAhBgNVBAMMGlNtYXJ0IEV4cGxvcmVyIEZUUCBUZXN0IENBMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEApB3tM83fyVTubkacpMJml8NtYhO95MLUbULlMOH5ioTcvwM2bSs5VVFOs9hT2e/j/YmFmyK1QfuaZ+rRAto94oORVgQ7rTIUTrvobJAFRqG8ar7XvSrX1dZMiUcXDK6iWYD7trDm4uAFZ0jXh57SBiQxQ0Brye0upX/acYA6XxeZeFS8gP1ps8KghE1p09+I/NWfqNL+2mMtNYCVaO3szow7QTezPuVPVCC9JcNC7X+MnvdYBDxLs2cvmQGx4kSrdBi21e3CziqX0kG46/NphDg4/pwKuB5YAdBXgKrgfhU43s0dlyLMf145CRhGFMRr4ST49Y0Ib915gci2Je6XZwIDAQABo2MwYTAdBgNVHQ4EFgQUNmQqOuBFYFJM/kzm94V/emEt6LkwHwYDVR0jBBgwFoAUNmQqOuBFYFJM/kzm94V/emEt6LkwDwYDVR0TAQH/BAUwAwEB/zAOBgNVHQ8BAf8EBAMCAQYwDQYJKoZIhvcNAQELBQADggEBAB012/pXhuou6Rsh0q6hI+y1Yo8jMs6c87rL1OAPTHptLrQs2LmF38x4biGatfXK4GwXYNxs4iZmn2RH+9lZUIbvcJ0wa49sspFDCILnkATqct0qHFuRFQY6gtvmFrH9LYlLuoMqSoeqt9h9ez8Q+dtBpiDoeM/8TGJDdW0yAvmkBsHrbRm8n8jXyWjIAZbAJRjQJOtPIh2oHiULyEZmof/pR221PHDEd0rFlGjIUXDPeN5fXmYNCDmiUwAJl8XrNl9n1+Runc8jM15LT31mXsv1MPXESGi1HhAOXw19qpdvyMzfziD167JFzgikMbeQaXNijrx/nFRJk1PsdWH4+nc=";
const TEST_LEAF_DER: &str = "MIIDXDCCAkSgAwIBAgIUWHuMJcN38PTITCkCtvIQZhkeooUwDQYJKoZIhvcNAQELBQAwJTEjMCEGA1UEAwwaU21hcnQgRXhwbG9yZXIgRlRQIFRlc3QgQ0EwIBcNMjYwNzEyMTgyMTI2WhgPMjEyNjA2MTgxODIxMjZaMBQxEjAQBgNVBAMMCWxvY2FsaG9zdDCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAM1dcpTzeAWLN2xIwfoPNLYyf8Eg5+/AX8T58dpgonZq4fYiP92YXbZYe/IFYSeHpxSdq6byR0TMb0+qpotacalypwXlazZoIMG6rO8KgRMVAsTLRIf22DKh9MXHVgr/tqKM5WXJmRxewvQRtRIs2+J8lhZwbC0HnNY/vO5WBC2PsI1V3m6kXrq0cYC292KZTWPxdHr8uek+sYjezxVuGVmTt8eLaqZUfVJ2UeR6AZ+NtqoW7vTz05C/Eb012BAS+bzacMCiBXwimpdwXRUvAjwGsRAQ3qHhgpffff2OQdJM/+tLB2L2wUzJBIVruDaTOKZtGwjbfkhVv6czKLElzRMCAwEAAaOBkjCBjzAMBgNVHRMBAf8EAjAAMA4GA1UdDwEB/wQEAwIFoDATBgNVHSUEDDAKBggrBgEFBQcDATAaBgNVHREEEzARgglsb2NhbGhvc3SHBH8AAAEwHQYDVR0OBBYEFIxN+EcoYOvoTZ330WrYUFI8U62xMB8GA1UdIwQYMBaAFDZkKjrgRWBSTP5M5veFf3phLei5MA0GCSqGSIb3DQEBCwUAA4IBAQBweKOh3cF412BfQ9CVowdVpNIsnjRTBztnfQYwgwV4zGcFYfSKupf14tIPnIK9F+Jef8CpqPS878KUspd4u1dN+oYfiIB4BPyZH7GkYey69jQ4+tSQCVeDEn7dGNOMDvDelaNFU5DUTLjEiccuEY7MvlaePtyckb/ipBf4eLHYJ5lD3XQt2fvcYTZgIbvSdMMBTwzV3HWA6m9jee2Tj5Zd8q108amm7bBw235Fk3uZeN6+SQ65MfEAlbXsf893zweJO8JVfqY3NO+ucd/9gyNoTg3TVYu/fC0UI2QXlAFAiZ7p33hpXBdbw3qYhufgrEqeT2TNoJIO/78xydnEYlNt";
const TEST_LEAF_KEY_DER: &str = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDNXXKU83gFizdsSMH6DzS2Mn/BIOfvwF/E+fHaYKJ2auH2Ij/dmF22WHvyBWEnh6cUnaum8kdEzG9PqqaLWnGpcqcF5Ws2aCDBuqzvCoETFQLEy0SH9tgyofTFx1YK/7aijOVlyZkcXsL0EbUSLNvifJYWcGwtB5zWP7zuVgQtj7CNVd5upF66tHGAtvdimU1j8XR6/LnpPrGI3s8VbhlZk7fHi2qmVH1SdlHkegGfjbaqFu7089OQvxG9NdgQEvm82nDAogV8IpqXcF0VLwI8BrEQEN6h4YKX3339jkHSTP/rSwdi9sFMyQSFa7g2kzimbRsI235IVb+nMyixJc0TAgMBAAECggEAWMqYAXW5BXCZUGyuzb6oVERGP0rKbTsYTTKiEoCojZmNxB0vztATaIUeZdhUlsJMh5naPw7OqJzZXbETW/oJXbGQLHjyX24rB4f+QEYi44y4izy1jzG3bUDf82lJtuyz2tkfT+CXnhAMq3lCeC7EDUs/m0kVRGzfrzSUq9mt6cJJiJEJb0TOlu7aRlhZfIr5PcPCJ4x0kUPtlUmOb7C1w9BnaVcA7WMkyzCQzg6lIK6iN2PCNY2WRUrjjZKKZTZXesovIBu828clGkcKA5WQpV/EjI/fRMWMkBTbCW3oiqerNORqs0NeGAnpCeVxXy/wR6AfytIB9N9pCHjjQ4jPaQKBgQDyT3xt6dxXbi7laQ9rZbVmaTaGq48v+goTGzR8yUt8dzuEnRJhXuw9J5ZYua9mBbjJCIiFoVVCypxl7O+miXuaoUQCWP7hVj1K428ib0XQk6zlWYzkm4hb+K3B87maFq410c8Xsj7DXPRNvXRfsrvEGUQYshPzqJW8LQyx1b4+qwKBgQDY959tpuX6wJAI1yVzN0f/oyBMRM8ZEU+rehdV4u0JvMN5hnNJrt7w7S3J1EkaqR2uKIPyztJUtDfnY1on3GeCS9330GGU963gR7OCxAScpoR8JZCog2goqZkt/mlUMsjnzy2L9dCTjiPEHQlDcxx63bM1Kwyc8VjnCrM5zUmLOQKBgQDqRNMmaU3w8cRBZJvV19XUF7Dx7vhXCEWpR0otw2hKA/T1N+9HWMDKN3XyfkQIPUv0gV2M5PhLxRwEp1jkCFQKohPguS5jqj9EIjOWdUJob/5fF39SntTtJrbHp94wDfGMczbn0BtCQqKobp0O0P0ckNj3j2Qe1UU/U8bMQLzYVQKBgGwgtBZ8h764us99EU/jLAGNtWntHNzcUL0fooOODR2+MhjdVZVSDg851IjyP+CGiaEi1edrBU1rZzTswaB96iP4VU3MTuVjrgbJFQBFWhsLrZkFS5t/qagiJZHTaYCpspA8IvHOdr0iqFZzNgukUXw2ArqrkqSgbvLt1TYoRc+ZAoGAHDpbCScU12C/18aG3yKA9ApjmbdtA/paJvdCIKWEBaDUYFS4CYdvynrqzZl348jtnjrBdK6zhr+uJLf0ndSoLr/Arj3fXf9Jjp5oO+ryif8OtXu+RYD8pbWa3tkb/9PwQtkrRPYyfSlKju0xoXEG71EUKA5UlsLPFmwGpXVHq90=";

#[derive(Default)]
struct FtpsEvents {
    auth: AtomicUsize,
    user: AtomicUsize,
    pass: AtomicUsize,
    binary: AtomicUsize,
}

fn decode(value: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .unwrap()
}

fn tls_configs() -> (Arc<rustls::ClientConfig>, Arc<ServerConfig>) {
    let ca = CertificateDer::from(decode(TEST_CA_DER));
    let leaf = CertificateDer::from(decode(TEST_LEAF_DER));
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(decode(TEST_LEAF_KEY_DER)));

    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let client = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let server = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![leaf], key)
        .unwrap();
    (Arc::new(client), Arc::new(server))
}

fn reply(stream: &mut impl Write, line: &str) -> io::Result<()> {
    stream.write_all(line.as_bytes())?;
    stream.write_all(b"\r\n")?;
    stream.flush()
}

fn read_command(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(
        line.trim_end_matches(['\r', '\n'])
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string(),
    ))
}

fn serve_explicit_ftps(
    mut stream: TcpStream,
    server_config: Arc<ServerConfig>,
    generation: usize,
    events: Arc<FtpsEvents>,
    replacement_noop: &mpsc::Sender<()>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    reply(&mut stream, "220 explicit FTPS ready")?;
    let mut plain_reader = BufReader::new(stream.try_clone()?);
    if read_command(&mut plain_reader)?.as_deref() != Some("AUTH") {
        return Err(io_err("fixture expected AUTH TLS"));
    }
    events.auth.fetch_add(1, Ordering::SeqCst);
    reply(&mut stream, "234 start TLS")?;
    drop(plain_reader);

    let connection = ServerConnection::new(server_config).map_err(io_err)?;
    let mut control = BufReader::new(StreamOwned::new(connection, stream));
    let mut passive = None;
    loop {
        let command = match read_command(&mut control) {
            Ok(Some(command)) => command,
            Ok(None) | Err(_) => return Ok(()),
        };
        match command.as_str() {
            "PBSZ" | "PROT" => reply(control.get_mut(), "200 protection ready")?,
            "USER" => {
                events.user.fetch_add(1, Ordering::SeqCst);
                reply(control.get_mut(), "331 password required")?;
            }
            "PASS" => {
                events.pass.fetch_add(1, Ordering::SeqCst);
                reply(control.get_mut(), "230 logged in")?;
            }
            "TYPE" => {
                events.binary.fetch_add(1, Ordering::SeqCst);
                reply(control.get_mut(), "200 binary")?;
            }
            "NOOP" if generation == 0 => {
                let _ = control.get_ref().sock.shutdown(Shutdown::Both);
                return Ok(());
            }
            "NOOP" => {
                reply(control.get_mut(), "200 alive")?;
                let _ = replacement_noop.send(());
            }
            "PASV" => {
                let listener = TcpListener::bind("127.0.0.1:0")?;
                let port = listener.local_addr()?.port();
                reply(
                    control.get_mut(),
                    &format!(
                        "227 Entering Passive Mode (127,0,0,1,{},{})",
                        port / 256,
                        port % 256
                    ),
                )?;
                passive = Some(listener);
            }
            "RETR" => {
                reply(control.get_mut(), "150 opening protected data")?;
                let listener = passive
                    .take()
                    .ok_or_else(|| io_err("fixture RETR without PASV"))?;
                let (data, _) = listener.accept()?;
                thread::sleep(Duration::from_millis(350));
                drop(data);
                let _ = reply(control.get_mut(), "426 data timeout");
            }
            "ABOR" => {
                let _ = reply(control.get_mut(), "226 aborted");
            }
            "PWD" => reply(control.get_mut(), "257 \"/\" is current directory")?,
            "QUIT" => {
                let _ = reply(control.get_mut(), "221 bye");
                return Ok(());
            }
            _ => reply(control.get_mut(), "500 unsupported")?,
        }
    }
}

#[test]
fn silent_server_greeting_is_cut_off_by_total_setup_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_millis(250));
    });
    let config = FtpUrl {
        secure: false,
        user: "test".to_string(),
        password: "secret".to_string(),
        host: address.ip().to_string(),
        port: address.port(),
        root: "/".to_string(),
    };
    let timing = FtpTiming {
        setup: Duration::from_millis(60),
        connect_attempt: Duration::from_millis(40),
        data_connect: Duration::from_millis(40),
        io: Duration::from_millis(80),
    };

    let started = Instant::now();
    let error = match connect_stream_with_timing(&config, timing, rustls_client_config()) {
        Ok(_) => panic!("silent greeting unexpectedly completed"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(1));
    server.join().unwrap();
}

#[test]
fn explicit_ftps_keepalive_reconnects_relogs_and_bounds_data_inactivity() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (client_tls, server_tls) = tls_configs();
    let events = Arc::new(FtpsEvents::default());
    let server_events = events.clone();
    let (replacement_noop_tx, replacement_noop_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        for generation in 0..2 {
            let (stream, _) = listener.accept().unwrap();
            serve_explicit_ftps(
                stream,
                server_tls.clone(),
                generation,
                server_events.clone(),
                &replacement_noop_tx,
            )
            .unwrap();
        }
    });
    let config = FtpUrl {
        secure: true,
        user: "test".to_string(),
        password: "secret".to_string(),
        host: address.ip().to_string(),
        port: address.port(),
        root: "/".to_string(),
    };
    let timing = FtpTiming {
        setup: Duration::from_secs(2),
        connect_attempt: Duration::from_millis(200),
        data_connect: Duration::from_millis(200),
        io: Duration::from_millis(80),
    };
    let stream = connect_stream_with_timing(&config, timing, client_tls.clone()).unwrap();
    let reconnect_config = config.clone();
    let reconnect_tls = client_tls.clone();
    let reconnect: FtpReconnect = Arc::new(move || {
        connect_stream_with_timing(&reconnect_config, timing, reconnect_tls.clone())
    });
    let connection = FtpConnection::new_with_timing(
        stream,
        reconnect,
        Duration::from_millis(20),
        Duration::from_millis(100),
    )
    .unwrap();

    replacement_noop_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("replacement FTPS session must receive an encrypted NOOP");
    assert_eq!(events.auth.load(Ordering::SeqCst), 2);
    assert_eq!(events.user.load(Ordering::SeqCst), 2);
    assert_eq!(events.pass.load(Ordering::SeqCst), 2);
    assert_eq!(events.binary.load(Ordering::SeqCst), 2);

    let mut reader = connection.open_reader("/blackhole").unwrap();
    let started = Instant::now();
    let error = reader.read(&mut [0u8; 1]).unwrap_err();
    assert!(matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ));
    assert!(started.elapsed() >= Duration::from_millis(40));
    assert!(started.elapsed() < Duration::from_secs(2));

    drop(reader);
    drop(connection);
    server.join().unwrap();
}
