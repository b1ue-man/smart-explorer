//! Ranges over the SSH agent (E13, U48-8): a range is one `Read` frame with
//! its `len`, and the agent's answer is cut at the end of the file. A
//! scripted agent records the frame it was sent.
use super::backend::AgentBackend;
use crate::agent_proto::{read_frame, write_frame, Frame, PROTO_VERSION};
use crate::vfs::Backend;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

const FILE: &[u8] = b"0123456789abcdefghij";

/// An agent that answers every `Read` from `FILE` like the real one
/// (`len` 0 = to the end) and keeps the frames it got.
fn agent() -> (AgentBackend, Arc<Mutex<Vec<(u64, u64)>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut reader = socket.try_clone().unwrap();
        let (hello, _) = read_frame(&mut reader).unwrap().unwrap();
        let ok = Frame::HelloOk {
            proto: PROTO_VERSION,
            version: "test".into(),
        };
        write_frame(&mut socket, hello, &ok).unwrap();
        while let Ok(Some((id, frame))) = read_frame(&mut reader) {
            let Frame::Read { offset, len, .. } = frame else {
                continue;
            };
            log.lock().unwrap().push((offset, len));
            let start = (offset as usize).min(FILE.len());
            let end = if len == 0 {
                FILE.len()
            } else {
                (start + len as usize).min(FILE.len())
            };
            write_frame(&mut socket, id, &Frame::Data(FILE[start..end].to_vec())).unwrap();
            write_frame(&mut socket, id, &Frame::End).unwrap();
        }
    });
    let client = TcpStream::connect(address).unwrap();
    let backend = AgentBackend::from_streams(
        Box::new(client.try_clone().unwrap()) as Box<dyn Read + Send>,
        Box::new(client) as Box<dyn Write + Send>,
        Arc::new(crate::vfs::LocalBackend::new("/")),
    )
    .unwrap();
    (backend, seen)
}

#[test]
fn speed_task_agent_range_reader_sends_len() {
    let (backend, seen) = agent();
    assert!(backend.supports_range_reads("/f"));
    let read = |offset: u64, len: u64| {
        let mut bytes = Vec::new();
        backend
            .open_read_range("/f", None, offset, len)
            .unwrap()
            .expect("the agent reads ranges")
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    };
    assert_eq!(read(5, 6), FILE[5..11]);
    assert_eq!(read(0, 4), FILE[..4], "also from the start of the file");
    // Fewer only at the end of the file.
    assert_eq!(read(15, 100), FILE[15..]);
    // An empty range is no `Read` (len 0 would mean the rest of the file).
    assert!(read(3, 0).is_empty());
    // The resume of a download still asks for the rest (len 0).
    let mut rest = Vec::new();
    backend
        .open_read_at("/f", None, 12)
        .unwrap()
        .unwrap()
        .read_to_end(&mut rest)
        .unwrap();
    assert_eq!(rest, FILE[12..]);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(5, 6), (0, 4), (15, 100), (12, 0)]
    );
}
