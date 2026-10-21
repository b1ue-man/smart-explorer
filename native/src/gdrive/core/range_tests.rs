//! Ranges over Google Drive (E13, U48-6): a part is its own request with a
//! bounded `Range` header, also from offset 0, and a server that ignores the
//! range turns the file into one stream.
use super::task_http::{Answer, Server};
use crate::vfs::Backend;
use serde_json::json;
use std::io::Read;

const CONTENT: &[u8] = b"0123456789abcdefghij";

fn media_server(honour: bool) -> Server {
    Server::start(move |request| {
        let range = request.header("range").and_then(|value| {
            let (first, last) = value.strip_prefix("bytes=")?.split_once('-')?;
            let first: usize = first.parse().ok()?;
            let last: usize = last.parse().unwrap_or(CONTENT.len() - 1);
            Some((first, last.min(CONTENT.len() - 1)))
        });
        let mut answer = match range {
            Some((first, last)) if honour => {
                let mut answer = Answer::status(206, json!(null)).header(
                    "Content-Range",
                    &format!("bytes {first}-{last}/{}", CONTENT.len()),
                );
                answer.body = CONTENT[first..=last].to_vec();
                answer
            }
            _ => Answer::status(200, json!(null)),
        };
        if answer.status == 200 {
            answer.body = CONTENT.to_vec();
        }
        answer
    })
}

fn backend_of(server: &Server) -> super::GDriveBackend {
    let backend = server.backend();
    backend
        .remember_path("a.bin", "a-id", Some("application/octet-stream"))
        .unwrap();
    backend
}

#[test]
fn speed_task_gdrive_range_reader_sets_bounded_range() {
    let server = media_server(true);
    let backend = backend_of(&server);
    assert!(backend.supports_range_reads("a.bin"));
    let read = |offset: u64, len: u64| {
        let mut bytes = Vec::new();
        backend
            .open_read_range("a.bin", Some("a-id"), offset, len)
            .unwrap()
            .expect("the server honours ranges")
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    };
    assert_eq!(read(5, 6), CONTENT[5..11]);
    // From offset 0 the range is still sent, with its end.
    assert_eq!(read(0, 4), CONTENT[..4]);
    let requests = server.requests();
    assert_eq!(requests[0].header("range"), Some("bytes=5-10"));
    assert_eq!(requests[1].header("range"), Some("bytes=0-3"));
    assert!(requests
        .iter()
        .all(|request| request.query("alt").as_deref() == Some("media")));
    // A resume still asks open-ended.
    let mut tail = Vec::new();
    backend
        .open_read_at("a.bin", Some("a-id"), 15)
        .unwrap()
        .unwrap()
        .read_to_end(&mut tail)
        .unwrap();
    assert_eq!(tail, CONTENT[15..]);
    assert_eq!(server.requests()[2].header("range"), Some("bytes=15-"));
}

#[test]
fn speed_task_gdrive_range_ignored_by_the_server_is_no_ranges() {
    let server = media_server(false);
    let backend = backend_of(&server);
    let opened = backend
        .open_read_range("a.bin", Some("a-id"), 5, 6)
        .unwrap();
    assert!(opened.is_none(), "a bounded read never skips to the offset");
}
