use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::api::{drive_request, send_retry};

fn read_request(stream: &TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    loop {
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            return;
        }
    }
}

fn respond(stream: &mut TcpStream, body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
}

#[test]
fn metadata_get_retries_when_body_drops_after_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let server_requests = requests.clone();
    let server = thread::spawn(move || {
        let (mut partial, _) = listener.accept().unwrap();
        partial
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        read_request(&partial);
        server_requests.fetch_add(1, Ordering::SeqCst);
        write!(
            partial,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 64\r\nConnection: close\r\n\r\n{{\"id\":"
        )
        .unwrap();
        partial.flush().unwrap();
        let _ = partial.shutdown(Shutdown::Both);

        let (mut replacement, _) = listener.accept().unwrap();
        replacement
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        read_request(&replacement);
        server_requests.fetch_add(1, Ordering::SeqCst);
        respond(&mut replacement, br#"{"id":"stable"}"#);
    });

    let url = format!("http://{address}/drive/v3/files/stable");
    let body = send_retry(|| drive_request(ureq::get(&url).timeout(Duration::from_secs(2)).call()))
        .unwrap();

    server.join().unwrap();
    assert_eq!(body, r#"{"id":"stable"}"#);
    assert_eq!(requests.load(Ordering::SeqCst), 2);
}
