use super::api::FOLDER_MIME;
use super::GDriveBackend;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

struct Request {
    method: String,
    target: String,
    body: Vec<u8>,
}

fn receive(listener: &TcpListener) -> (TcpStream, Request) {
    let (mut stream, _) = listener.accept().unwrap();
    let mut raw = Vec::new();
    while !raw.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        raw.push(byte[0]);
    }
    let headers = String::from_utf8(raw[..raw.len() - 4].to_vec()).unwrap();
    let mut first = headers.lines().next().unwrap().split_whitespace();
    let method = first.next().unwrap().to_string();
    let target = first.next().unwrap().to_string();
    let length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse::<usize>().unwrap())
        .unwrap_or(0);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).unwrap();
    (
        stream,
        Request {
            method,
            target,
            body,
        },
    )
}

fn reply(stream: &mut TcpStream, body: &str) {
    reply_status(stream, "200 OK", body);
}

fn reply_status(stream: &mut TcpStream, status: &str, body: &str) {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
}

fn listener() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    (listener, base)
}

fn assert_folder_create(request: &Request, expected_id: &str) {
    assert_eq!(request.method, "POST");
    assert!(request.target.contains("/files?fields=id"));
    let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert_eq!(body["id"], expected_id);
    assert_eq!(body["name"], "folder");
    assert_eq!(body["mimeType"], FOLDER_MIME);
    assert_eq!(body["parents"], serde_json::json!(["root"]));
}

fn folder_state(id: &str) -> String {
    serde_json::json!({
        "id": id,
        "name": "folder",
        "mimeType": FOLDER_MIME,
        "parents": ["root"],
        "trashed": false
    })
    .to_string()
}

#[test]
fn rename_commit_then_drop_reconciles_exact_id_without_replaying_patch() {
    let (listener, base) = listener();
    let server = thread::spawn(move || {
        let mut methods = Vec::new();

        let (mut stream, request) = receive(&listener);
        methods.push(request.method.clone());
        assert_eq!(request.method, "GET");
        assert!(request.target.contains("files?q="));
        reply(
            &mut stream,
            r#"{"files":[{"id":"file-id","name":"old.txt","mimeType":"text/plain","size":"1","md5Checksum":"abc","parents":["root"],"trashed":false}]}"#,
        );

        let (mut stream, request) = receive(&listener);
        methods.push(request.method.clone());
        assert_eq!(request.method, "GET");
        reply(&mut stream, r#"{"files":[]}"#);

        let (stream, request) = receive(&listener);
        methods.push(request.method.clone());
        assert_eq!(request.method, "PATCH");
        assert!(request.target.contains("/files/file-id?fields=id"));
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["name"], "new.txt");
        drop(stream); // Drive committed the PATCH, but its ACK was lost.

        let (mut stream, request) = receive(&listener);
        methods.push(request.method.clone());
        assert_eq!(request.method, "GET");
        assert!(request.target.contains("/files/file-id?fields=id"));
        reply(
            &mut stream,
            r#"{"id":"file-id","name":"new.txt","mimeType":"text/plain","parents":["root"],"trashed":false}"#,
        );

        let (mut stream, request) = receive(&listener);
        methods.push(request.method.clone());
        assert_eq!(request.method, "GET");
        reply(
            &mut stream,
            r#"{"files":[{"id":"file-id","name":"new.txt","mimeType":"text/plain","size":"1","md5Checksum":"abc","parents":["root"],"trashed":false}]}"#,
        );
        methods
    });

    let backend = GDriveBackend::test_backend(&base);
    backend
        .remember_path("old.txt", "file-id", Some("text/plain"))
        .unwrap();
    backend.listed_guard().unwrap().insert(String::new());
    backend.rename_serialized("old.txt", "new.txt").unwrap();
    let methods = server.join().unwrap();

    assert_eq!(
        methods.iter().filter(|method| *method == "PATCH").count(),
        1
    );
    assert_eq!(backend.cached_id("old.txt").unwrap(), None);
    assert_eq!(
        backend.cached_id("new.txt").unwrap().as_deref(),
        Some("file-id")
    );
    assert!(backend.listed_guard().unwrap().contains(""));
}

#[test]
fn trash_commit_then_503_reconciles_exact_id_without_replaying_patch() {
    let (listener, base) = listener();
    let server = thread::spawn(move || {
        let mut methods = Vec::new();
        let (mut stream, request) = receive(&listener);
        methods.push(request.method.clone());
        assert_eq!(request.method, "PATCH");
        assert!(request.target.contains("/files/file-id?fields=id,trashed"));
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["trashed"], true);
        reply_status(
            &mut stream,
            "503 Service Unavailable",
            r#"{"error":{"message":"response failed after commit"}}"#,
        );

        let (mut stream, request) = receive(&listener);
        methods.push(request.method.clone());
        assert_eq!(request.method, "GET");
        assert!(request.target.contains("/files/file-id?fields=id,trashed"));
        reply(&mut stream, r#"{"id":"file-id","trashed":true}"#);
        methods
    });

    let backend = GDriveBackend::test_backend(&base);
    backend
        .remember_path("gone.txt", "file-id", Some("text/plain"))
        .unwrap();
    backend.listed_guard().unwrap().insert(String::new());
    backend.trash_path_id("gone.txt", "file-id").unwrap();
    let methods = server.join().unwrap();

    assert_eq!(
        methods.iter().filter(|method| *method == "PATCH").count(),
        1
    );
    assert_eq!(backend.cached_id("gone.txt").unwrap(), None);
    assert!(backend.listed_guard().unwrap().contains(""));
}

#[test]
fn trash_blackholed_ack_times_out_and_reconciles_without_replay() {
    let (listener, base) = listener();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut methods = Vec::new();
        let (stream, request) = receive(&listener);
        methods.push(request.method.clone());
        assert_eq!(request.method, "PATCH");
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["trashed"], true);
        // Keep the committed request's TCP response blackholed until the
        // client deadline fires and exact-ID reconciliation completes.
        let held = thread::spawn(move || {
            release_rx.recv().unwrap();
            drop(stream);
        });

        let (mut stream, request) = receive(&listener);
        methods.push(request.method.clone());
        assert_eq!(request.method, "GET");
        assert!(request.target.contains("/files/file-id?fields=id,trashed"));
        reply(&mut stream, r#"{"id":"file-id","trashed":true}"#);
        held.join().unwrap();
        methods
    });

    let backend = GDriveBackend::test_backend_with_timeout(&base, Duration::from_millis(50));
    backend
        .remember_path("gone.txt", "file-id", Some("text/plain"))
        .unwrap();
    backend.trash_path_id("gone.txt", "file-id").unwrap();
    release_tx.send(()).unwrap();
    let methods = server.join().unwrap();

    assert_eq!(
        methods.iter().filter(|method| *method == "PATCH").count(),
        1
    );
    assert_eq!(backend.cached_id("gone.txt").unwrap(), None);
}

#[test]
fn folder_create_commit_then_drop_uses_reserved_id_and_updates_parent_snapshot() {
    let (listener, base) = listener();
    let server = thread::spawn(move || {
        let mut methods = Vec::new();
        let (mut stream, request) = receive(&listener);
        methods.push(request.method.clone());
        assert_eq!(request.method, "GET");
        assert!(request.target.contains("/files/generateIds?"));
        reply(&mut stream, r#"{"ids":["folder-id"]}"#);

        let (stream, request) = receive(&listener);
        methods.push(request.method.clone());
        assert_eq!(request.method, "POST");
        assert!(request.target.contains("/files?fields=id"));
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["id"], "folder-id");
        assert_eq!(body["name"], "folder");
        assert_eq!(body["mimeType"], FOLDER_MIME);
        assert_eq!(body["parents"], serde_json::json!(["root"]));
        drop(stream);

        let (mut stream, request) = receive(&listener);
        methods.push(request.method.clone());
        assert_eq!(request.method, "GET");
        assert!(request.target.contains("/files/folder-id?fields=id"));
        reply(
            &mut stream,
            &serde_json::json!({
                "id": "folder-id",
                "name": "folder",
                "mimeType": FOLDER_MIME,
                "parents": ["root"],
                "trashed": false
            })
            .to_string(),
        );
        methods
    });

    let backend = GDriveBackend::test_backend(&base);
    backend.listed_guard().unwrap().insert(String::new());
    assert_eq!(backend.ensure_dir("folder").unwrap(), "folder-id");
    let methods = server.join().unwrap();

    assert_eq!(methods.iter().filter(|method| *method == "POST").count(), 1);
    assert_eq!(
        backend.cached_id("folder").unwrap().as_deref(),
        Some("folder-id")
    );
    assert_eq!(backend.mime_of("folder").as_deref(), Some(FOLDER_MIME));
    let listed = backend.listed_guard().unwrap();
    assert!(listed.contains(""));
    assert!(listed.contains("folder"));
}

#[test]
fn ambiguous_folder_retry_reuses_the_reserved_id_and_never_generates_another() {
    let (listener, base) = listener();
    let server = thread::spawn(move || {
        let mut requests = Vec::new();

        let (mut stream, request) = receive(&listener);
        requests.push((request.method.clone(), request.target.clone()));
        assert_eq!(request.method, "GET");
        assert!(request.target.contains("/files/generateIds?"));
        reply(&mut stream, r#"{"ids":["folder-id"]}"#);

        let (stream, request) = receive(&listener);
        requests.push((request.method.clone(), request.target.clone()));
        assert_folder_create(&request, "folder-id");
        drop(stream); // The create may have committed, but its response was lost.

        let (mut stream, request) = receive(&listener);
        requests.push((request.method.clone(), request.target.clone()));
        assert_eq!(request.method, "GET");
        assert!(request.target.contains("/files/folder-id?fields=id"));
        reply_status(&mut stream, "404 Not Found", r#"{"error":{}}"#);

        // A later call must reconcile the durable exact ID before doing
        // anything else. If it is not visible yet, Drive explicitly permits a
        // same-ID retry; 409 means the first request did commit.
        let (mut stream, request) = receive(&listener);
        requests.push((request.method.clone(), request.target.clone()));
        assert_eq!(request.method, "GET");
        assert!(request.target.contains("/files/folder-id?fields=id"));
        reply_status(&mut stream, "404 Not Found", r#"{"error":{}}"#);

        let (mut stream, request) = receive(&listener);
        requests.push((request.method.clone(), request.target.clone()));
        assert_folder_create(&request, "folder-id");
        reply_status(
            &mut stream,
            "409 Conflict",
            r#"{"error":{"message":"ID already exists"}}"#,
        );

        let (mut stream, request) = receive(&listener);
        requests.push((request.method.clone(), request.target.clone()));
        assert_eq!(request.method, "GET");
        assert!(request.target.contains("/files/folder-id?fields=id"));
        reply(&mut stream, &folder_state("folder-id"));
        requests
    });

    let backend = GDriveBackend::test_backend(&base);
    backend.listed_guard().unwrap().insert(String::new());
    let first = backend.ensure_dir("folder").unwrap_err();
    assert!(first.to_string().contains("ambiguous completion"));
    assert_eq!(
        backend.pending_folder_create("folder").unwrap().unwrap().id,
        "folder-id"
    );

    assert_eq!(backend.ensure_dir("folder").unwrap(), "folder-id");
    assert!(backend.pending_folder_create("folder").unwrap().is_none());
    let requests = server.join().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|(_, target)| target.contains("/files/generateIds?"))
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|(method, target)| method == "POST" && target.contains("/files?fields=id"))
            .count(),
        2
    );
}

#[test]
fn pending_folder_reservation_survives_backend_restart_and_reconciles_without_post() {
    let (listener, base) = listener();
    let server = thread::spawn(move || {
        let mut methods = Vec::new();

        let (mut stream, request) = receive(&listener);
        methods.push(request.method.clone());
        assert!(request.target.contains("/files/generateIds?"));
        reply(&mut stream, r#"{"ids":["durable-folder-id"]}"#);

        let (stream, request) = receive(&listener);
        methods.push(request.method.clone());
        assert_folder_create(&request, "durable-folder-id");
        drop(stream);

        let (mut stream, request) = receive(&listener);
        methods.push(request.method.clone());
        assert_eq!(request.method, "GET");
        reply_status(&mut stream, "404 Not Found", r#"{"error":{}}"#);

        // The restarted backend loads the pending record and verifies its ID.
        let (mut stream, request) = receive(&listener);
        methods.push(request.method.clone());
        assert_eq!(request.method, "GET");
        assert!(request
            .target
            .contains("/files/durable-folder-id?fields=id"));
        reply(&mut stream, &folder_state("durable-folder-id"));
        methods
    });

    let storage = tempfile::tempdir().unwrap();
    let first = GDriveBackend::test_backend_with_pending_dir(
        &base,
        Duration::from_secs(3),
        storage.path().to_path_buf(),
    );
    first.listed_guard().unwrap().insert(String::new());
    assert!(first.ensure_dir("folder").is_err());
    assert_eq!(std::fs::read_dir(storage.path()).unwrap().count(), 1);
    drop(first);

    let restarted = GDriveBackend::test_backend_with_pending_dir(
        &base,
        Duration::from_secs(3),
        storage.path().to_path_buf(),
    );
    assert_eq!(restarted.ensure_dir("folder").unwrap(), "durable-folder-id");
    assert_eq!(std::fs::read_dir(storage.path()).unwrap().count(), 0);
    let methods = server.join().unwrap();
    assert_eq!(methods.iter().filter(|method| *method == "POST").count(), 1);
}
