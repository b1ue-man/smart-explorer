use super::*;
use std::io::{BufRead, BufReader};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

fn read_request(reader: &mut BufReader<TcpStream>) -> String {
    let mut content_length = 0usize;
    let mut request_line = String::new();
    loop {
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if request_line.is_empty() {
            request_line = line.trim_end().to_string();
        }
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body).unwrap();
    request_line
}

fn respond_multistatus(stream: &mut TcpStream) {
    let body = b"<multistatus/>";
    write!(
        stream,
        "HTTP/1.1 207 Multi-Status\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
}

fn respond_ok(stream: &mut TcpStream, body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
}

fn backend_for(base: String) -> WebdavBackend {
    backend_for_timeout(base, Duration::from_secs(3))
}

fn backend_for_timeout(base: String, timeout: Duration) -> WebdavBackend {
    WebdavBackend {
        base: base.clone(),
        root: "/".to_string(),
        auth: String::new(),
        agent: ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(3))
            .timeout_read(timeout)
            .timeout_write(timeout)
            .build(),
        mutation_agent: ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(3))
            .timeout_read(Duration::from_secs(3))
            .timeout_write(Duration::from_secs(3))
            .redirects(0)
            .max_idle_connections(0)
            .build(),
        url: base.clone(),
        identity: format!("webdav:{base}"),
    }
}

#[test]
fn propfind_retries_when_body_drops_after_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let server_requests = requests.clone();
    let server = thread::spawn(move || {
        let (mut partial, _) = listener.accept().unwrap();
        partial
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut partial_reader = BufReader::new(partial.try_clone().unwrap());
        assert!(read_request(&mut partial_reader).starts_with("PROPFIND "));
        server_requests.fetch_add(1, Ordering::SeqCst);
        write!(
            partial,
            "HTTP/1.1 207 Multi-Status\r\nContent-Length: 64\r\nConnection: close\r\n\r\n<multistatus"
        )
        .unwrap();
        partial.flush().unwrap();
        let _ = partial.shutdown(Shutdown::Both);

        let (mut replacement, _) = listener.accept().unwrap();
        replacement
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut replacement_reader = BufReader::new(replacement.try_clone().unwrap());
        assert!(read_request(&mut replacement_reader).starts_with("PROPFIND "));
        server_requests.fetch_add(1, Ordering::SeqCst);
        respond_multistatus(&mut replacement);
    });

    let backend = backend_for(format!("http://{address}"));
    assert_eq!(backend.propfind("/", "0").unwrap(), "<multistatus/>");
    server.join().unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}

#[test]
fn propfind_body_blackhole_stops_after_one_bounded_retry() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let server_requests = requests.clone();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut held = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            assert!(read_request(&mut reader).starts_with("PROPFIND "));
            server_requests.fetch_add(1, Ordering::SeqCst);
            write!(
                stream,
                "HTTP/1.1 207 Multi-Status\r\nContent-Length: 64\r\nConnection: keep-alive\r\n\r\n"
            )
            .unwrap();
            stream.flush().unwrap();
            held.push(stream);
        }
        release_rx.recv().unwrap();
        drop(held);
    });

    let backend = backend_for_timeout(format!("http://{address}"), Duration::from_millis(150));
    let started = std::time::Instant::now();
    let error = backend.propfind("/", "0").unwrap_err();
    let elapsed = started.elapsed();
    release_tx.send(()).unwrap();
    server.join().unwrap();

    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(elapsed < Duration::from_secs(2), "elapsed: {elapsed:?}");
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}

#[test]
fn propfind_reconnects_after_ambiguous_stale_pool_close() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let server_requests = requests.clone();
    let server = thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        first
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut first_reader = BufReader::new(first.try_clone().unwrap());
        read_request(&mut first_reader);
        server_requests.fetch_add(1, Ordering::SeqCst);
        respond_multistatus(&mut first);

        // Consume the next safe PROPFIND completely, then lose its response.
        // This forces the backend's explicit retry path rather than ureq's
        // pre-request stale-socket detection.
        read_request(&mut first_reader);
        server_requests.fetch_add(1, Ordering::SeqCst);
        let _ = first.shutdown(Shutdown::Both);

        let (mut replacement, _) = listener.accept().unwrap();
        replacement
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut replacement_reader = BufReader::new(replacement.try_clone().unwrap());
        read_request(&mut replacement_reader);
        server_requests.fetch_add(1, Ordering::SeqCst);
        respond_multistatus(&mut replacement);
    });

    let base = format!("http://{address}");
    let backend = backend_for(base);

    assert_eq!(backend.propfind("/", "0").unwrap(), "<multistatus/>");
    assert_eq!(backend.propfind("/", "0").unwrap(), "<multistatus/>");
    server.join().unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 3);
}

#[test]
fn get_reconnects_before_exposing_body_after_stale_pool_close() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let server_requests = requests.clone();
    let server = thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        first
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut first_reader = BufReader::new(first.try_clone().unwrap());
        assert!(read_request(&mut first_reader).starts_with("GET "));
        server_requests.fetch_add(1, Ordering::SeqCst);
        respond_ok(&mut first, b"prime");

        // The request reached the server, but no response body reached the
        // caller. GET can therefore be established once more on a fresh
        // connection without duplicating a mutation or delivered bytes.
        assert!(read_request(&mut first_reader).starts_with("GET "));
        server_requests.fetch_add(1, Ordering::SeqCst);
        let _ = first.shutdown(Shutdown::Both);

        let (mut replacement, _) = listener.accept().unwrap();
        replacement
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut replacement_reader = BufReader::new(replacement.try_clone().unwrap());
        assert!(read_request(&mut replacement_reader).starts_with("GET "));
        server_requests.fetch_add(1, Ordering::SeqCst);
        respond_ok(&mut replacement, b"payload");
    });

    let backend = backend_for(format!("http://{address}"));
    let mut prime = String::new();
    backend
        .open_read("/file")
        .unwrap()
        .read_to_string(&mut prime)
        .unwrap();
    assert_eq!(prime, "prime");

    let mut payload = String::new();
    backend
        .open_read("/file")
        .unwrap()
        .read_to_string(&mut payload)
        .unwrap();
    assert_eq!(payload, "payload");
    server.join().unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 3);
}

#[test]
fn delete_response_loss_is_not_replayed() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (committed_tx, committed_rx) = mpsc::channel();
    let (inspect_tx, inspect_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        assert!(read_request(&mut reader).starts_with("DELETE "));
        let _ = stream.shutdown(Shutdown::Both);
        committed_tx.send(()).unwrap();
        inspect_rx.recv().unwrap();
        listener.set_nonblocking(true).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    });
    let backend = backend_for(format!("http://{address}"));

    let result = backend.remove_file("/committed");
    committed_rx.recv().unwrap();
    assert!(result.is_err());
    inspect_tx.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn mutation_redirect_is_not_followed_or_reported_as_success() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (inspect_tx, inspect_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        assert!(read_request(&mut reader).starts_with("DELETE "));
        write!(
            stream,
            "HTTP/1.1 302 Found\r\nLocation: /canonical\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        stream.flush().unwrap();
        inspect_rx.recv().unwrap();
        listener.set_nonblocking(true).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    });
    let backend = backend_for(format!("http://{address}"));

    let error = backend.remove_file("/redirected").unwrap_err();
    assert!(error.to_string().contains("HTTP status 302"));
    inspect_tx.send(()).unwrap();
    server.join().unwrap();
}

#[test]
fn put_redirect_is_terminal_and_never_followed() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (inspect_tx, inspect_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        assert!(read_request(&mut reader).starts_with("PUT "));
        write!(
            stream,
            "HTTP/1.1 302 Found\r\nLocation: /canonical\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        stream.flush().unwrap();
        inspect_rx.recv().unwrap();
        listener.set_nonblocking(true).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    });
    let backend = backend_for(format!("http://{address}"));
    let mut writer = backend.open_write("/redirected").unwrap();
    writer.write_all(b"payload").unwrap();

    let error = writer.flush().unwrap_err();
    assert!(error.to_string().contains("HTTP status 302"));
    assert!(writer.flush().is_err());
    inspect_tx.send(()).unwrap();
    server.join().unwrap();
}
