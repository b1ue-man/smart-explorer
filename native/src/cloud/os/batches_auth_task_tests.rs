//! B1: production coordinator with an isolated credential/config snapshot.
//! The task suite must serialize environment-mutating fixtures (--test-threads=1).
use super::*;
use std::io::{Read, Write};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

struct Fixture {
    _root: tempfile::TempDir,
    previous: Option<std::ffi::OsString>,
    snapshot: Option<crate::cloud::TestSessionSnapshot>,
}
fn environment_key() -> &'static str {
    if cfg!(windows) { "APPDATA" } else { "XDG_DATA_HOME" }
}
impl Fixture {
    fn new() -> Self {
        let previous = std::env::var_os(environment_key());
        let root = tempfile::tempdir().unwrap();
        std::env::set_var(environment_key(), root.path());
        let snapshot = crate::cloud::test_session_snapshot(Provider::GDrive).unwrap();
        let fixture = Self { _root: root, previous, snapshot: Some(snapshot) };
        crate::cloud::test_reset_session(Provider::GDrive).unwrap();
        shared::save_config(Provider::GDrive, &ClientConfig {
            client_id: "fixture-client".into(), client_secret: "fixture-secret".into(),
        }).unwrap();
        store_import(Provider::GDrive, "refresh-0").unwrap();
        fixture
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(snapshot) = self.snapshot.take() {
            crate::cloud::test_restore_session(Provider::GDrive, snapshot).unwrap();
        }
        match &self.previous {
            Some(previous) => std::env::set_var(environment_key(), previous),
            None => std::env::remove_var(environment_key()),
        }
    }
}

struct ProviderFixture {
    url: String,
    worker: Option<std::thread::JoinHandle<Vec<String>>>,
}
impl ProviderFixture {
    fn start(replies: Vec<(u16, &'static str)>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/token", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in replies {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "coordinator did not send expected request");
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("fixture accept: {error}"),
                    }
                };
                stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                stream.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
                let mut request = Vec::new();
                loop {
                    let mut bytes = [0; 2048];
                    let count = stream.read(&mut bytes).unwrap();
                    assert_ne!(count, 0);
                    request.extend_from_slice(&bytes[..count]);
                    assert!(request.len() <= 64 * 1024);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = std::str::from_utf8(&request[..end]).unwrap();
                        let length: usize = headers.lines().find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse().unwrap())
                        }).unwrap();
                        if request.len() >= end + 4 + length {
                            requests.push(std::str::from_utf8(&request[end + 4..end + 4 + length]).unwrap().to_owned());
                            break;
                        }
                    }
                }
                write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        Self { url, worker: Some(worker) }
    }
    fn requests(mut self) -> Vec<String> { self.worker.take().unwrap().join().unwrap() }
}
impl Drop for ProviderFixture {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}
fn identify(_: &str) -> Result<String, AuthError> { Ok("fixture-principal".into()) }

#[test]
fn batches_task_oauth_concurrent_refreshes_consume_committed_rotation_in_order() {
    let _fixture = Fixture::new();
    let server = ProviderFixture::start(vec![
        (200, r#"{"access_token":"access-1","refresh_token":"refresh-1"}"#),
        (200, r#"{"access_token":"access-2","refresh_token":"refresh-2"}"#),
    ]);
    let start = Arc::new(Barrier::new(3));
    let results = std::thread::scope(|scope| {
        let first_start = start.clone();
        let url = &server.url;
        let first = scope.spawn(move || {
            first_start.wait();
            refresh(Provider::GDrive, url, Some("fixture-principal"), identify)
        });
        let second_start = start.clone();
        let second = scope.spawn(move || {
            second_start.wait();
            refresh(Provider::GDrive, url, Some("fixture-principal"), identify)
        });
        start.wait();
        [first.join().unwrap().unwrap(), second.join().unwrap().unwrap()]
    });
    let mut tokens: Vec<_> = results.iter().map(|tokens| tokens.refresh_token.as_str()).collect();
    tokens.sort();
    assert_eq!(tokens, ["refresh-1", "refresh-2"]);
    let requests = server.requests();
    assert!(requests[0].contains("refresh_token=refresh-0"));
    assert!(requests[1].contains("refresh_token=refresh-1"));
    assert!(requests.iter().all(|body| body.contains("client_id=fixture-client") && body.contains("client_secret=fixture-secret")));
    let record = store::read(Provider::GDrive).unwrap().unwrap();
    assert_eq!(record.refresh_token, "refresh-2");
    assert_eq!(record.account_key.as_deref(), Some("fixture-principal"));
    assert_eq!(record.state, AuthState::Connected);
}

#[test]
fn batches_task_oauth_identity_outage_preserves_rotated_token_without_returning_access() {
    let _fixture = Fixture::new();
    let server = ProviderFixture::start(vec![
        (200, r#"{"access_token":"must-not-escape","refresh_token":"rotated"}"#),
        (200, r#"{"access_token":"verified"}"#),
    ]);
    let generation = store::read(Provider::GDrive).unwrap().unwrap().generation;
    let error = refresh(Provider::GDrive, &server.url, None,
        |_| Err(AuthError::new(AuthState::IdentityUnverified))).err().unwrap();
    assert_eq!(error.state, AuthState::IdentityUnverified);
    let record = store::read(Provider::GDrive).unwrap().unwrap();
    assert_eq!(record.refresh_token, "rotated");
    assert!(record.account_key.is_none());
    assert!(record.generation > generation);
    let tokens = refresh(Provider::GDrive, &server.url, Some("fixture-principal"), identify).unwrap();
    assert_eq!(tokens.refresh_token, "rotated");
    assert_eq!(tokens.access_token, "verified");
    assert!(server.requests()[1].contains("refresh_token=rotated"));
}

#[test]
fn batches_task_oauth_invalid_grant_stops_retry_and_diagnostics_exclude_provider_secrets() {
    let _fixture = Fixture::new();
    let server = ProviderFixture::start(vec![
        (400, r#"{"error":"invalid_grant","error_description":"secret-provider-body"}"#),
    ]);
    let error = refresh(Provider::GDrive, &server.url, None, identify).err().unwrap();
    assert_eq!(error.state, AuthState::ReauthorizeRequired);
    assert!(!error.to_string().contains("secret-provider-body"));
    server.requests();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/unexpected", listener.local_addr().unwrap());
    assert_eq!(refresh(Provider::GDrive, &url, None, |_| panic!("must not identify blocked grant")).err().unwrap().state, AuthState::ReauthorizeRequired);
    assert_eq!(listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    let status = serde_json::to_string(&status(Provider::GDrive)).unwrap();
    for secret in ["refresh-0", "fixture-secret", "secret-provider-body"] { assert!(!status.contains(secret)); }
    assert_eq!(store::read(Provider::GDrive).unwrap().unwrap().refresh_token, "refresh-0");
}

#[test]
fn batches_task_oauth_disconnect_supersedes_pending_login_and_keeps_confirmed_principal() {
    let _fixture = Fixture::new();
    let server = ProviderFixture::start(vec![(200, r#"{"access_token":"access"}"#)]);
    refresh(Provider::GDrive, &server.url, None, identify).unwrap();
    server.requests();
    let (config, generation) = begin_authorize(Provider::GDrive).unwrap();
    disconnect(Provider::GDrive).unwrap();
    let error = finish_authorize(Provider::GDrive, &config, generation,
        Tokens { access_token: "late-access".into(), refresh_token: "late-refresh".into(), expires_at: 0 },
        &std::sync::atomic::AtomicBool::new(false)).err().unwrap();
    assert_eq!(error.state, AuthState::OperationSuperseded);
    let record = store::read(Provider::GDrive).unwrap().unwrap();
    assert!(record.refresh_token.is_empty());
    assert_eq!(record.account_key.as_deref(), Some("fixture-principal"));
    assert_eq!(validate_account(Provider::GDrive, "fixture-principal").unwrap_err().state, AuthState::Disconnected);
}

#[test]
fn batches_task_oauth_account_mismatch_never_rebinds_existing_principal() {
    let _fixture = Fixture::new();
    let server = ProviderFixture::start(vec![
        (200, r#"{"access_token":"original-access"}"#),
        (200, r#"{"access_token":"foreign-access","refresh_token":"foreign-refresh"}"#),
    ]);
    refresh(Provider::GDrive, &server.url, None, identify).unwrap();
    let error = refresh(Provider::GDrive, &server.url, Some("fixture-principal"),
        |_| Ok("foreign-principal".into())).err().unwrap();
    assert_eq!(error.state, AuthState::IdentityConflict);
    server.requests();
    let record = store::read(Provider::GDrive).unwrap().unwrap();
    assert_eq!(record.account_key.as_deref(), Some("fixture-principal"));
    assert_eq!(record.refresh_token, "refresh-0");
    assert_eq!(record.state, AuthState::IdentityConflict);
    assert_eq!(validate_account(Provider::GDrive, "fixture-principal").unwrap_err().state, AuthState::IdentityConflict);
}

#[test]
fn batches_task_oauth_client_roundtrip_invalidates_old_browser_generation() {
    let _fixture = Fixture::new();
    let server = ProviderFixture::start(vec![(200, r#"{"access_token":"access"}"#)]);
    refresh(Provider::GDrive, &server.url, None, identify).unwrap();
    server.requests();
    let (original, generation) = begin_authorize(Provider::GDrive).unwrap();
    shared::save_config(Provider::GDrive, &ClientConfig {
        client_id: "replacement-client".into(), client_secret: "replacement-secret".into(),
    }).unwrap();
    assert_eq!(validate_account(Provider::GDrive, "fixture-principal").unwrap_err().state, AuthState::ClientChanged);
    shared::save_config(Provider::GDrive, &original).unwrap();
    assert_eq!(finish_authorize(Provider::GDrive, &original, generation,
        Tokens { access_token: "stale-access".into(), refresh_token: "stale-refresh".into(), expires_at: 0 },
        &std::sync::atomic::AtomicBool::new(false)).err().unwrap().state, AuthState::OperationSuperseded);
    let record = store::read(Provider::GDrive).unwrap().unwrap();
    assert_eq!(record.refresh_token, "refresh-0");
    assert_eq!(record.account_key.as_deref(), Some("fixture-principal"));
}
