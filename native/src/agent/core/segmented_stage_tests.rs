//! The client side of ranged uploads and LZ4 blocks over the SSH agent: a
//! server without the labels gets none of the new frames, a server with them
//! is served range by range, and `DataLz4` blocks are unpacked.
use super::super::backend::AgentBackend;
use crate::agent_proto::{
    pack_lz4, read_frame, serve, server_version, write_frame, Frame, MAX_RAW, PROTO_VERSION,
};
use crate::vfs::Backend;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

const MIB: usize = 1024 * 1024;

fn text(len: usize) -> Vec<u8> {
    b"ranged uploads and compressed blocks, "
        .iter()
        .copied()
        .cycle()
        .take(len)
        .collect()
}

type Seen = Arc<Mutex<Vec<Frame>>>;
type Script = Box<dyn FnMut(u64, &Frame, &mut TcpStream) + Send>;

/// An agent that announces `version`, records every frame after the hello
/// and answers with `script`.
fn scripted(version: &str, mut script: Script) -> (AgentBackend, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let version = version.to_string();
    std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("accept");
        let mut reader = socket.try_clone().expect("clone");
        let (hello, _) = read_frame(&mut reader).expect("hello").expect("hello");
        let ok = Frame::HelloOk {
            proto: PROTO_VERSION,
            version,
        };
        write_frame(&mut socket, hello, &ok).expect("hello ok");
        while let Ok(Some((id, frame))) = read_frame(&mut reader) {
            script(id, &frame, &mut socket);
            log.lock().expect("log").push(frame);
        }
    });
    let client = TcpStream::connect(address).expect("connect");
    let backend = AgentBackend::from_streams(
        Box::new(client.try_clone().expect("clone")) as Box<dyn Read + Send>,
        Box::new(client) as Box<dyn Write + Send>,
        Arc::new(crate::vfs::LocalBackend::new("/")),
    )
    .expect("backend");
    (backend, seen)
}

/// An agent that takes every write like the real one: ready, credit back
/// for every data frame (a credit connection stops the sender after
/// `CREDIT_INITIAL` bytes otherwise), then `Ok` at `End`.
fn accepting() -> Script {
    Box::new(|id, frame, socket| match frame {
        Frame::Write(_) | Frame::WriteNew(_) | Frame::WriteAt { .. } => {
            write_frame(socket, id, &Frame::Progress { done: 0, total: 0 }).expect("ready");
        }
        Frame::Data(data) => {
            let bytes = data.len() as u64;
            write_frame(socket, id, &Frame::Credit { bytes }).expect("credit");
        }
        Frame::End => write_frame(socket, id, &Frame::Ok).expect("ok"),
        _ => {}
    })
}

const STAGE_HINT: &str = "/data/big.bin.se-upload-0123456789abcdef";

#[test]
fn speed_task_agent_old_server_without_label_uses_one_stream() {
    let (backend, seen) = scripted(
        "0.5.100+sync-links-v1.credit-v1.stage-v1.slots-64",
        accepting(),
    );
    assert!(
        backend
            .open_segmented_stage(STAGE_HINT, 20 * MIB as u64)
            .expect("no error")
            .is_none(),
        "no stage without the label: the file goes as one stream"
    );
    let mut writer = backend.open_write_new("/data/new.bin").expect("write");
    writer.write_all(&text(MIB)).expect("data");
    writer.flush().expect("commit");
    drop(writer);

    let frames = seen.lock().expect("log");
    assert!(frames
        .iter()
        .any(|frame| matches!(frame, Frame::WriteNew(_))));
    assert!(
        frames.iter().all(|frame| !matches!(
            frame,
            Frame::SegStageOpen { .. }
                | Frame::WriteAt { .. }
                | Frame::SegStageFinish { .. }
                | Frame::DataLz4(_)
                | Frame::ReadLz4 { .. }
        )),
        "{frames:?}"
    );
    assert!(frames.iter().any(|frame| matches!(frame, Frame::Data(_))));
}

#[test]
fn speed_task_lz4_agent_old_server_gets_plain_data() {
    let script: Script = Box::new(|id, frame, socket| match frame {
        Frame::Read { .. } => {
            write_frame(socket, id, &Frame::Data(text(1000))).expect("data");
            write_frame(socket, id, &Frame::End).expect("end");
        }
        other => accepting()(id, other, socket),
    });
    let (backend, seen) = scripted("0.5.100+credit-v1.stage-v1.slots-64", script);
    let mut bytes = Vec::new();
    backend
        .open_read("/data/f")
        .expect("read")
        .read_to_end(&mut bytes)
        .expect("bytes");
    assert_eq!(bytes, text(1000));
    let frames = seen.lock().expect("log");
    assert!(frames
        .iter()
        .any(|frame| matches!(frame, Frame::Read { .. })));
    assert!(frames
        .iter()
        .all(|frame| !matches!(frame, Frame::ReadLz4 { .. } | Frame::DataLz4(_))));
}

#[test]
fn speed_task_agent_client_unpacks_lz4_blocks_of_a_server_with_the_label() {
    let raw = text(MAX_RAW);
    let block = pack_lz4(&raw).expect("text shrinks");
    let answer = block.clone();
    let script: Script = Box::new(move |id, frame, socket| {
        if let Frame::ReadLz4 { .. } = frame {
            write_frame(socket, id, &Frame::DataLz4(answer.clone())).expect("block");
            write_frame(socket, id, &Frame::Data(b"tail".to_vec())).expect("plain");
            write_frame(socket, id, &Frame::End).expect("end");
        }
    });
    let (backend, seen) = scripted("0.5.200+credit-v1.stage-v1.lz4-v1.slots-64", script);
    let mut bytes = Vec::new();
    backend
        .open_read_range("/data/f", None, 0, MAX_RAW as u64 + 4)
        .expect("read")
        .expect("ranges")
        .read_to_end(&mut bytes)
        .expect("bytes");
    assert_eq!(&bytes[..MAX_RAW], &raw[..]);
    assert_eq!(&bytes[MAX_RAW..], b"tail");
    assert!(
        seen.lock()
            .expect("log")
            .iter()
            .any(|frame| matches!(frame, Frame::ReadLz4 { offset: 0, .. })),
        "a client that unpacks says so with ReadLz4"
    );
}

/// The real agent loop served over a socket.
fn real_agent() -> AgentBackend {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    std::thread::spawn(move || {
        let (socket, _) = listener.accept().expect("accept");
        let read = socket.try_clone().expect("clone");
        let _ = serve(read, socket);
    });
    let client = TcpStream::connect(address).expect("connect");
    AgentBackend::from_streams(
        Box::new(client.try_clone().expect("clone")) as Box<dyn Read + Send>,
        Box::new(client) as Box<dyn Write + Send>,
        Arc::new(crate::vfs::LocalBackend::new("/")),
    )
    .expect("backend")
}

#[test]
fn speed_task_agent_segmented_stage_end_to_end() {
    let announced = server_version(true);
    assert!(announced.contains("range-v1"), "{announced}");
    let dir = tempfile::tempdir().expect("temp dir");
    let destination = dir.path().join("big.bin").to_string_lossy().into_owned();
    let size = 20 * MIB + 321;
    let body: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();
    let backend = real_agent();

    let hint = format!("{destination}.se-upload-0123456789abcdef");
    let stage = backend
        .open_segmented_stage(&hint, size as u64)
        .expect("open")
        .expect("the agent announced range-v1");
    let name = stage.stage_path().to_string();
    assert!(
        name.starts_with(&format!("{destination}.se-upload-")),
        "{name}"
    );
    assert_ne!(name, hint, "the agent picks the name, not the client");

    let bounds = [0, 8 * MIB, 16 * MIB, size];
    std::thread::scope(|scope| {
        let stage = &*stage;
        let body = &body;
        for pair in bounds.windows(2).rev() {
            let (from, to) = (pair[0], pair[1]);
            scope.spawn(move || {
                let mut writer = stage
                    .segment(from as u64, (to - from) as u64)
                    .expect("range");
                writer.write_all(&body[from..to]).expect("data");
                writer.flush().expect("range complete");
            });
        }
    });
    stage.finish().expect("the stage is complete");

    assert!(!std::path::Path::new(&destination).exists());
    backend
        .promote_staged_no_replace(&name, &destination)
        .expect("publish without replacing");
    assert_eq!(std::fs::read(&destination).expect("file"), body);
    assert!(!std::path::Path::new(&name).exists());
}

#[test]
fn speed_task_agent_short_range_is_not_committed() {
    let dir = tempfile::tempdir().expect("temp dir");
    let destination = dir.path().join("short.bin").to_string_lossy().into_owned();
    let backend = real_agent();
    let hint = format!("{destination}.se-upload-0123456789abcdef");
    let stage = backend
        .open_segmented_stage(&hint, 1000)
        .expect("open")
        .expect("stage");
    let mut writer = stage.segment(0, 1000).expect("range");
    writer.write_all(&[1; 600]).expect("data");
    assert_eq!(
        writer.flush().expect_err("600 of 1000").kind(),
        std::io::ErrorKind::InvalidData
    );
    assert!(stage.finish().is_err(), "the range never arrived");
}
