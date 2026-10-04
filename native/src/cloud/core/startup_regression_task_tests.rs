//! The real refresh path against a local HTTP provider on the remote runner.
use super::*;

struct Fixture {
    root: tempfile::TempDir,
    previous: Option<std::ffi::OsString>,
    token: Option<String>,
}

impl Fixture {
    fn new() -> Self {
        #[cfg(windows)]
        let key = "APPDATA";
        #[cfg(not(windows))]
        let key = "XDG_DATA_HOME";
        let previous = std::env::var_os(key);
        let root = tempfile::tempdir().unwrap();
        std::env::set_var(key, root.path());
        let token = refresh_token_checked(Provider::GDrive).unwrap();
        Self {
            root,
            previous,
            token,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        match &self.token {
            Some(token) => store_refresh_token(Provider::GDrive, token).unwrap(),
            None => disconnect(Provider::GDrive).unwrap(),
        }
        #[cfg(windows)]
        let key = "APPDATA";
        #[cfg(not(windows))]
        let key = "XDG_DATA_HOME";
        match &self.previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

#[test]
fn startup_regression_task_refresh_sends_preserved_client_id_and_token() {
    let fixture = Fixture::new();
    save_config(
        Provider::GDrive,
        &ClientConfig {
            client_id: "stored.apps.googleusercontent.com".into(),
            client_secret: "saved-secret".into(),
        },
    )
    .unwrap();
    store_refresh_token(Provider::GDrive, "persisted-refresh").unwrap();
    let path = fixture.root.path().join("smart_explorer/cloud/gdrive.cfg");
    let before = std::fs::read(&path).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/token", listener.local_addr().unwrap());
    let provider = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(std::time::Instant::now() < deadline, "missing OAuth request");
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("OAuth provider accept failed: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 2048];
        loop {
            let n = stream.read(&mut buffer).unwrap();
            assert_ne!(n, 0);
            request.extend_from_slice(&buffer[..n]);
            assert!(request.len() <= 65536);
            if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&request[..end]);
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap();
                if request.len() >= end + 4 + length {
                    let body = std::str::from_utf8(&request[end + 4..end + 4 + length]).unwrap();
                    assert!(body.contains("client_id=stored.apps.googleusercontent.com"));
                    assert!(body.contains("client_secret=saved-secret"));
                    assert!(body.contains("refresh_token=persisted-refresh"));
                    assert!(body.contains("grant_type=refresh_token"));
                    break;
                }
            }
        }
        let body = r#"{"access_token":"new-access","expires_in":3600}"#;
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let tokens = refresh_access_from(Provider::GDrive, &url);
    provider.join().unwrap();
    let tokens = tokens.unwrap();
    assert_eq!(tokens.access_token, "new-access");
    assert_eq!(tokens.refresh_token, "persisted-refresh");
    assert_eq!(
        refresh_token_checked(Provider::GDrive).unwrap().as_deref(),
        Some("persisted-refresh")
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn startup_regression_task_bad_config_stops_before_any_http_request() {
    let fixture = Fixture::new();
    let path = fixture.root.path().join("smart_explorer/cloud/gdrive.cfg");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, [0xff]).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/token", listener.local_addr().unwrap());
    let error = refresh_access_from(Provider::GDrive, &url).err().unwrap();
    assert!(error.contains("Cloud-Konfiguration"));
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    std::fs::write(&path, b"client_id=\n").unwrap();
    let error = refresh_access_from(Provider::GDrive, &url).err().unwrap();
    assert!(error.contains("Client-ID"));
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
