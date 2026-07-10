use super::*;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::thread;

fn token() -> io::Result<String> {
    Ok("test-token".to_string())
}

fn receive(listener: &TcpListener) -> (TcpStream, String, Vec<u8>) {
    let (mut stream, _) = listener.accept().unwrap();
    let mut raw = Vec::new();
    while !raw.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        raw.push(byte[0]);
    }
    let headers = String::from_utf8(raw[..raw.len() - 4].to_vec()).unwrap();
    let length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse::<usize>().unwrap())
        .unwrap_or(0);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).unwrap();
    (stream, headers, body)
}

fn reply(stream: &mut TcpStream, status: &str, headers: &str, body: &str) {
    write!(
        stream,
        "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
}

#[test]
fn chunk_boundary_rotation_truncated_completion_and_empty_query() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server_base = base.clone();
    let server = thread::spawn(move || {
        let (mut first, headers, body) = receive(&listener);
        assert!(headers.contains(&format!(
            "Content-Range: bytes 0-{}/{}",
            CHUNK_SIZE - 1,
            CHUNK_SIZE + 1
        )));
        assert_eq!(body.len(), CHUNK_SIZE);
        reply(
            &mut first,
            "308 Resume Incomplete",
            &format!(
                "Range: bytes=0-{}\r\nLocation: {server_base}/second\r\n",
                CHUNK_SIZE - 1
            ),
            "",
        );
        let (mut second, headers, body) = receive(&listener);
        assert!(headers.contains(&format!(
            "Content-Range: bytes {0}-{0}/{1}",
            CHUNK_SIZE,
            CHUNK_SIZE + 1
        )));
        assert_eq!(body, [0]);
        reply(&mut second, "201 Created", "", "{");

        let (empty, headers, body) = receive(&listener);
        assert!(headers.starts_with("PUT /empty "));
        assert!(!headers.to_ascii_lowercase().contains("content-range:"));
        assert!(body.is_empty());
        drop(empty);
        let (mut query, headers, body) = receive(&listener);
        assert!(headers.contains("Content-Range: bytes */0"));
        assert!(body.is_empty());
        reply(&mut query, "200 OK", "", "");
    });

    let mut spool = tempfile::tempfile().unwrap();
    spool.set_len((CHUNK_SIZE + 1) as u64).unwrap();
    let done = upload(
        &format!("{base}/first"),
        &mut spool,
        (CHUNK_SIZE + 1) as u64,
        "known-id",
        token,
        token,
    )
    .unwrap();
    assert_eq!(done, Completion::VerifyExpected);
    let mut empty = tempfile::tempfile().unwrap();
    let done = upload(
        &format!("{base}/empty"),
        &mut empty,
        0,
        "empty-id",
        token,
        token,
    )
    .unwrap();
    assert_eq!(done, Completion::VerifyExpected);
    server.join().unwrap();
}

#[test]
fn refreshes_once_on_unauthorized_without_losing_the_chunk() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stale, headers, body) = receive(&listener);
        assert!(headers
            .to_ascii_lowercase()
            .contains("authorization: bearer stale"));
        assert!(headers.contains("Content-Range: bytes 0-0/1"));
        assert_eq!(body, [0]);
        reply(&mut stale, "401 Unauthorized", "", "expired");

        let (mut fresh, headers, body) = receive(&listener);
        assert!(headers
            .to_ascii_lowercase()
            .contains("authorization: bearer fresh"));
        assert!(headers.contains("Content-Range: bytes 0-0/1"));
        assert_eq!(body, [0]);
        reply(&mut fresh, "200 OK", "", r#"{"id":"known-id"}"#);
    });

    let mut gets = 0;
    let mut refreshes = 0;
    let mut spool = tempfile::tempfile().unwrap();
    spool.set_len(1).unwrap();
    let done = upload(
        &format!("{base}/token"),
        &mut spool,
        1,
        "known-id",
        || {
            gets += 1;
            Ok("stale".to_string())
        },
        || {
            refreshes += 1;
            Ok("fresh".to_string())
        },
    )
    .unwrap();
    assert_eq!(done, Completion::Confirmed);
    assert_eq!(gets, 1);
    assert_eq!(refreshes, 1);
    server.join().unwrap();
}

#[test]
fn range_completion_and_rate_limit_parsing_are_strict() {
    let complete = ureq::Response::new(200, "OK", r#"{"id":"known-id"}"#).unwrap();
    assert_eq!(
        completion(complete, "known-id").unwrap(),
        Completion::Confirmed
    );
    let wrong = ureq::Response::new(201, "Created", r#"{"id":"other"}"#).unwrap();
    assert!(completion(wrong, "known-id").is_err());
    let rate = ureq::Response::new(403, "Forbidden", "rateLimitExceeded").unwrap();
    assert!(matches!(
        request_failure(ureq::Error::Status(403, rate)),
        RequestFailure::Transient(_)
    ));
    let absent = ureq::Response::new(308, "Resume Incomplete", "").unwrap();
    assert_eq!(confirmed_offset(&absent, 100, 0, Some(43)).unwrap(), 0);
    assert!(confirmed_offset(&absent, 100, 1, None).is_err());
    let response = |range: &str| {
        format!("HTTP/1.1 308 X\r\nRange: {range}\r\n\r\n")
            .parse::<ureq::Response>()
            .unwrap()
    };
    assert_eq!(
        confirmed_offset(&response("bytes=0-42"), 100, 0, Some(43)).unwrap(),
        43
    );
    assert_eq!(
        confirmed_offset(&response("bytes=0-99"), 100, 0, None).unwrap(),
        100
    );
    assert!(confirmed_offset(&response("bytes=0-43"), 100, 0, Some(43)).is_err());
    for range in ["bytes 0-42", "bytes=1-42", "bytes=0-100", "bytes=0-4,8"] {
        assert!(confirmed_offset(&response(range), 100, 0, None).is_err());
    }
}
