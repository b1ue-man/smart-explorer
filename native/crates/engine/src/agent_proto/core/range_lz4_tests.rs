//! Ranged uploads on the agent (`range-v1`) and LZ4 content blocks
//! (`lz4-v1`): the stage table behind the frames, the frame layouts, the
//! block format and what the credit counts.
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::agent_proto::server_session::ServerSession;
use crate::agent_proto::{
    credit_cost, pack_lz4, pack_raw, raw_len, read_frame, serve, server_version,
    server_version_with, unpack, write_frame, BlockSender, Frame, ServerFeatures, CHUNK,
    CREDIT_INITIAL, MAX_RAW,
};
use crate::transfer::compress_policy::{Policy, METER_WINDOW};

const MIB: usize = 1024 * 1024;

fn text(len: usize) -> Vec<u8> {
    let words = b"the quick brown fox jumps over the lazy dog and then does it again ";
    words.iter().copied().cycle().take(len).collect()
}

/// Bytes zlib-style compressors cannot shrink: a xorshift stream.
fn random(len: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

/// One agent connection served in a thread.
struct Remote {
    writer: TcpStream,
    reader: TcpStream,
    server: Option<std::thread::JoinHandle<()>>,
    next_id: u64,
}

impl Remote {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let server = std::thread::spawn(move || {
            let (socket, _) = listener.accept().expect("accept");
            let read = socket.try_clone().expect("clone");
            let _ = serve(read, socket);
        });
        let writer = TcpStream::connect(address).expect("connect");
        let reader = writer.try_clone().expect("clone");
        reader
            .set_read_timeout(Some(Duration::from_secs(20)))
            .expect("timeout");
        Self {
            writer,
            reader,
            server: Some(server),
            next_id: 1,
        }
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn send(&mut self, id: u64, frame: &Frame) {
        write_frame(&mut self.writer, id, frame).expect("send");
    }

    fn next(&mut self) -> Frame {
        read_frame(&mut self.reader)
            .expect("read")
            .expect("a reply")
            .1
    }

    fn open(&mut self, destination: &str, size: u64) -> Result<String, String> {
        let id = self.id();
        self.send(
            id,
            &Frame::SegStageOpen {
                path: destination.to_string(),
                size,
            },
        );
        match self.next() {
            Frame::SegStageReady { stage } => Ok(stage),
            Frame::Err(error) => Err(error),
            other => panic!("unexpected reply {other:?}"),
        }
    }

    /// `WriteAt` with the given data frames; the error of the first refusal.
    fn write_at(
        &mut self,
        stage: &str,
        offset: u64,
        len: u64,
        frames: Vec<Frame>,
    ) -> Result<(), String> {
        let id = self.id();
        self.send(
            id,
            &Frame::WriteAt {
                stage: stage.to_string(),
                offset,
                len,
            },
        );
        match self.next() {
            Frame::Progress { .. } => {}
            Frame::Err(error) => return Err(error),
            other => panic!("unexpected reply {other:?}"),
        }
        for frame in frames {
            self.send(id, &frame);
        }
        self.send(id, &Frame::End);
        match self.next() {
            Frame::Ok => Ok(()),
            Frame::Err(error) => Err(error),
            other => panic!("unexpected reply {other:?}"),
        }
    }

    fn finish(&mut self, stage: &str) -> Result<(), String> {
        let id = self.id();
        self.send(
            id,
            &Frame::SegStageFinish {
                stage: stage.to_string(),
            },
        );
        match self.next() {
            Frame::Ok => Ok(()),
            Frame::Err(error) => Err(error),
            other => panic!("unexpected reply {other:?}"),
        }
    }

    /// The client goes away; the server loop ends and cleans up.
    fn hang_up(mut self) {
        let _ = self.writer.shutdown(std::net::Shutdown::Both);
        if let Some(server) = self.server.take() {
            server.join().expect("server thread");
        }
    }
}

fn data(bytes: &[u8]) -> Vec<Frame> {
    bytes
        .chunks(CHUNK)
        .map(|chunk| Frame::Data(chunk.to_vec()))
        .collect()
}

fn destination(dir: &tempfile::TempDir, name: &str) -> String {
    dir.path().join(name).to_string_lossy().into_owned()
}

#[test]
fn speed_task_agent_stage_open_creates_an_exclusive_sparse_stage() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = destination(&dir, "big.bin");
    let mut remote = Remote::start();
    let first = remote.open(&target, 3 * MIB as u64 + 5).expect("open");
    let second = remote.open(&target, 10).expect("open again");

    assert_ne!(first, second, "the host picks a new random name each time");
    for stage in [&first, &second] {
        assert!(
            stage.starts_with(&format!("{target}.se-upload-")),
            "{stage}"
        );
        let name = stage.rsplit(['/', '\\']).next().expect("name");
        assert!(crate::vfs::is_unique_stage(name), "{name}");
    }
    assert_eq!(
        std::fs::metadata(&first).expect("stage").len(),
        3 * MIB as u64 + 5,
        "the stage has its final length from the start"
    );
    assert!(
        !std::path::Path::new(&target).exists(),
        "nothing is written under the destination name"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&first)
            .expect("stage")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "a private stage");
    }

    // Like `WriteNew`, the stage makes the missing plain folders of its
    // destination (a tree copy writes into folders that do not exist yet).
    let missing = destination(&dir, "no/such/folder/file.bin");
    let made = remote.open(&missing, 1).expect("missing folders are made");
    assert!(made.starts_with(&format!("{missing}.se-upload-")), "{made}");
    assert!(std::path::Path::new(&made).is_file(), "{made}");
    // A file in the way is no folder.
    std::fs::write(dir.path().join("plain.txt"), b"x").expect("file");
    let below_file = destination(&dir, "plain.txt/file.bin");
    assert!(
        remote.open(&below_file, 1).is_err(),
        "the folder of a stage is never a file"
    );
    #[cfg(unix)]
    {
        let real = dir.path().join("real");
        std::fs::create_dir(&real).expect("folder");
        std::os::unix::fs::symlink(&real, dir.path().join("link")).expect("link");
        let through_link = destination(&dir, "link/file.bin");
        assert!(
            remote.open(&through_link, 1).is_err(),
            "a stage is never made below a link"
        );
    }
    remote.hang_up();
}

#[test]
fn speed_task_agent_write_at_rejects_overlap_and_out_of_range() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut remote = Remote::start();
    let stage = remote
        .open(&destination(&dir, "f.bin"), 4 * MIB as u64)
        .expect("open");
    let payload = text(MIB);

    remote
        .write_at(&stage, 0, MIB as u64, data(&payload))
        .expect("first range");
    let overlap = remote
        .write_at(&stage, MIB as u64 / 2, MIB as u64, Vec::new())
        .expect_err("overlaps the finished range");
    assert!(overlap.contains("überlappt"), "{overlap}");
    for (offset, len) in [
        (4 * MIB as u64 - 10, 20),
        (4 * MIB as u64, 1),
        (u64::MAX, 2),
    ] {
        remote
            .write_at(&stage, offset, len, Vec::new())
            .expect_err("outside the stage");
    }
    remote
        .write_at(&stage, 0, 0, Vec::new())
        .expect_err("an empty range");

    // Fewer or more bytes than announced leave the range free.
    let short = remote
        .write_at(&stage, MIB as u64, 100, vec![Frame::Data(vec![1; 99])])
        .expect_err("99 of 100 bytes");
    assert!(!short.is_empty());
    let long = remote
        .write_at(&stage, MIB as u64, 100, vec![Frame::Data(vec![1; 101])])
        .expect_err("101 of 100 bytes");
    assert!(!long.is_empty());
    remote
        .write_at(&stage, MIB as u64, 100, vec![Frame::Data(vec![9; 100])])
        .expect("exactly 100 bytes");
    let on_disk = std::fs::read(&stage).expect("stage");
    assert_eq!(&on_disk[..MIB], &payload[..]);
    assert_eq!(&on_disk[MIB..MIB + 100], &[9u8; 100][..]);
    remote.hang_up();
}

#[test]
fn speed_task_agent_stage_finish_requires_full_coverage() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut remote = Remote::start();
    let size = 2 * MIB;
    let stage = remote
        .open(&destination(&dir, "g.bin"), size as u64)
        .expect("open");
    let body = text(size);
    remote
        .write_at(&stage, 0, MIB as u64, data(&body[..MIB]))
        .expect("first half");
    assert!(remote.finish(&stage).is_err(), "the second half is missing");
    remote
        .write_at(&stage, MIB as u64, MIB as u64, data(&body[MIB..]))
        .expect("second half");
    remote.finish(&stage).expect("everything is there");
    assert_eq!(std::fs::read(&stage).expect("stage"), body);
    assert!(
        remote
            .write_at(&stage, 0, 1, vec![Frame::Data(vec![0])])
            .is_err(),
        "a finished stage takes no more ranges"
    );
    remote.hang_up();
    assert!(
        std::path::Path::new(&stage).exists(),
        "a finished stage stays for publication"
    );
}

#[test]
fn speed_task_agent_stage_belongs_to_the_opening_connection_and_goes_with_it() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut owner = Remote::start();
    let mut stranger = Remote::start();
    let stage = owner
        .open(&destination(&dir, "h.bin"), MIB as u64)
        .expect("open");
    let refused = stranger
        .write_at(&stage, 0, 1, vec![Frame::Data(vec![1])])
        .expect_err("another connection");
    assert!(refused.contains("Unbekannte Stufe"), "{refused}");
    assert!(stranger.finish(&stage).is_err());
    assert!(std::path::Path::new(&stage).exists());
    stranger.hang_up();

    owner.hang_up();
    assert!(
        !std::path::Path::new(&stage).exists(),
        "an unfinished stage is removed when its connection ends"
    );
}

#[test]
fn speed_task_agent_stage_table_is_limited_by_the_credit_request_limit() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut remote = Remote::start();
    let mut stages = Vec::new();
    for index in 0..crate::agent_proto::CREDIT_REQUEST_LIMIT {
        let name = format!("many{index}.bin");
        stages.push(remote.open(&destination(&dir, &name), 1).expect("open"));
    }
    let refused = remote
        .open(&destination(&dir, "one-too-many.bin"), 1)
        .expect_err("the table is full");
    assert!(
        crate::agent_proto::parse_busy(&refused).is_some(),
        "{refused}"
    );
    remote.hang_up();
    assert!(stages
        .iter()
        .all(|stage| !std::path::Path::new(stage).exists()));
}

#[test]
fn speed_task_agent_range_frames_are_charged_by_credit() {
    // The control frames of a ranged upload are free; its data is not.
    for frame in [
        Frame::SegStageOpen {
            path: "/a".into(),
            size: 1,
        },
        Frame::SegStageReady { stage: "/s".into() },
        Frame::WriteAt {
            stage: "/s".into(),
            offset: 0,
            len: 1,
        },
        Frame::SegStageFinish { stage: "/s".into() },
    ] {
        assert_eq!(credit_cost(&frame), 0, "{frame:?}");
        let bytes = frame.encode(7).expect("encode");
        assert_eq!(Frame::decode(&bytes).expect("decode"), (7, frame));
    }
    assert!(credit_cost(&Frame::Data(vec![0; 1000])) > 0);

    // A WriteAt upload runs under the credit it was granted: four blocks of
    // 256 KiB raw use the 1 MiB up to the byte, the fifth is a violation —
    // although every block is only a few KiB on the wire.
    let session = ServerSession::new(Arc::new(Mutex::new(Box::new(Vec::<u8>::new()))));
    assert!(session.route(0, Frame::Credit { bytes: 0 }).is_none());
    let request = Frame::WriteAt {
        stage: "/s".into(),
        offset: 0,
        len: 8 * MIB as u64,
    };
    let context = session.open(5, &request);
    assert!(context.inbound.is_some(), "WriteAt is an upload");
    let block = pack_lz4(&text(MAX_RAW)).expect("text shrinks");
    assert!(block.len() < MAX_RAW / 10);
    assert_eq!(CREDIT_INITIAL as usize, 4 * MAX_RAW);
    for _ in 0..4 {
        assert!(session.route(5, Frame::DataLz4(block.clone())).is_none());
        assert!(!context.cancel.load(Ordering::Relaxed));
    }
    assert!(session.route(5, Frame::DataLz4(block)).is_none());
    assert!(context.cancel.load(Ordering::Relaxed), "past its credit");
}

#[test]
fn speed_task_agent_unknown_frame_tag_is_an_error() {
    let mut raw = Vec::new();
    raw.extend_from_slice(&1u64.to_le_bytes());
    raw.push(69);
    assert_eq!(
        Frame::decode(&raw).expect_err("tag 69 is unknown").kind(),
        ErrorKind::InvalidData
    );
}

#[test]
fn speed_task_agent_labels_range_and_lz4() {
    let agent = ServerFeatures::parse(&server_version(true));
    assert!(agent.ranges);
    assert_eq!(agent.lz4, !crate::agent_proto::lz4_switched_off());
    let service = ServerFeatures::parse(&format!("{} worker", server_version_with(true, 12)));
    assert!(
        !service.ranges && !service.lz4,
        "the background service serves neither of the two"
    );
    let old = ServerFeatures::parse("0.5.100+sync-links-v1.credit-v1.stage-v1.slots-64");
    assert!(!old.ranges && !old.lz4, "an old agent announces neither");
}

#[test]
fn speed_task_agent_lz4_frames_roundtrip_on_the_wire() {
    let block = pack_lz4(&text(MAX_RAW)).expect("packs");
    for frame in [
        Frame::DataLz4(block),
        Frame::DataLz4(pack_raw(&random(1000))),
        Frame::ReadLz4 {
            path: "/data/x".into(),
            offset: 5,
            len: 77,
        },
    ] {
        let bytes = frame.encode(9).expect("encode");
        assert_eq!(Frame::decode(&bytes).expect("decode"), (9, frame));
    }
    let oversized = Frame::DataLz4(vec![0; MAX_RAW + 6]);
    assert_eq!(
        oversized.encode(1).expect_err("too long").kind(),
        ErrorKind::InvalidData
    );
}

#[path = "lz4_block_tests.rs"]
mod blocks;
