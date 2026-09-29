//! Hardening of the agent's batch and stage handlers: changed lengths,
//! cancelable server copies, bounded free frames, batch counts and the
//! exact stage names a discard may remove.
use super::batch_get::handle_get_batch;
use super::server_session::ServerSession;
use super::session::Sink;
use super::stage_ops::{copy_to_stage, discard_stage, is_discardable_stage};
use super::{
    clip_text, read_frame, BatchEntry, BatchItem, Frame, BATCH_MAX_FILES, ITEM_PATH_MAX,
    ITEM_TEXT_MAX,
};
use std::io::{self, ErrorKind, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A sink whose frames the test reads back.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn sink(&self) -> Sink {
        Arc::new(Mutex::new(Box::new(self.clone())))
    }

    fn frames(&self) -> Vec<Frame> {
        let mut cursor = io::Cursor::new(self.0.lock().unwrap().clone());
        let mut frames = Vec::new();
        while let Some((_, frame)) = read_frame(&mut cursor).unwrap() {
            frames.push(frame);
        }
        frames
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "se_agent_bounds_{label}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[cfg(unix)]
#[test]
fn transfer_engine_task_batch_download_fails_items_whose_length_changed() {
    let root = temp_dir("length");
    std::fs::write(root.join("five.txt"), b"12345").unwrap();
    let path = root.join("five.txt").to_string_lossy().into_owned();
    let items: Vec<BatchItem> = [5, 3, 9]
        .into_iter()
        .map(|size| BatchItem {
            path: path.clone(),
            id: None,
            size,
        })
        .collect();
    let captured = Captured::default();
    handle_get_batch(&captured.sink(), 1, &items, &AtomicBool::new(false)).unwrap();
    let frames = captured.frames();
    assert_eq!(frames.len(), 6, "{frames:?}");
    assert_eq!(frames[0], Frame::ItemBegin { index: 0, size: 5 });
    assert_eq!(frames[1], Frame::Data(b"12345".to_vec()));
    assert_eq!(
        frames[2],
        Frame::ItemEnd {
            index: 0,
            error: None
        }
    );
    // Grown and shrunk since the listing: no byte of them is sent, so the
    // batch never exceeds the bytes it announced.
    assert!(matches!(&frames[3], Frame::ItemFailed { index: 1, .. }));
    assert!(matches!(&frames[4], Frame::ItemFailed { index: 2, .. }));
    assert_eq!(frames[5], Frame::End);
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn transfer_engine_task_server_copy_removes_only_the_stage_it_created() {
    let root = temp_dir("copy");
    std::fs::write(root.join("src.bin"), vec![7u8; 1024]).unwrap();
    let src = root.join("src.bin").to_string_lossy().into_owned();
    let stage = root.join("dst.bin.se-upload-0000000000000001");
    let stage_text = stage.to_string_lossy().into_owned();

    let error = copy_to_stage(&src, &stage_text, 1024, &AtomicBool::new(true)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Interrupted);
    assert!(!stage.exists(), "a canceled copy removes its own stage");

    std::fs::write(&stage, b"someone else").unwrap();
    let error = copy_to_stage(&src, &stage_text, 1024, &AtomicBool::new(false)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&stage).unwrap(), b"someone else");
    std::fs::remove_file(&stage).unwrap();

    assert!(copy_to_stage(&src, &stage_text, 1000, &AtomicBool::new(false)).is_err());
    assert!(!stage.exists(), "a wrong length creates nothing");

    let copied = copy_to_stage(&src, &stage_text, 1024, &AtomicBool::new(false)).unwrap();
    assert_eq!(copied, 1024);
    assert_eq!(std::fs::read(&stage).unwrap(), vec![7u8; 1024]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn transfer_engine_task_item_texts_are_bounded_on_both_codec_ends() {
    let fits = Frame::ItemEnd {
        index: 0,
        error: Some("x".repeat(ITEM_TEXT_MAX)),
    };
    assert_eq!(Frame::decode(&fits.encode(1).unwrap()).unwrap().1, fits);
    let path = Frame::ItemPublished {
        index: 0,
        path: "p".repeat(ITEM_PATH_MAX),
    };
    assert_eq!(Frame::decode(&path.encode(1).unwrap()).unwrap().1, path);
    for frame in [
        Frame::ItemEnd {
            index: 0,
            error: Some("x".repeat(ITEM_TEXT_MAX + 1)),
        },
        Frame::ItemFailed {
            index: 0,
            message: "x".repeat(ITEM_TEXT_MAX + 1),
        },
        Frame::ItemPublished {
            index: 0,
            path: "p".repeat(ITEM_PATH_MAX + 1),
        },
    ] {
        assert_eq!(frame.encode(1).unwrap_err().kind(), ErrorKind::InvalidData);
    }
    // A peer that encodes an oversize trailer anyway is refused on decode.
    let mut raw = Vec::new();
    raw.extend_from_slice(&1u64.to_le_bytes());
    raw.push(38);
    raw.extend_from_slice(&0u32.to_le_bytes());
    raw.push(1);
    raw.extend_from_slice(&((ITEM_TEXT_MAX + 1) as u32).to_le_bytes());
    raw.resize(raw.len() + ITEM_TEXT_MAX + 1, b'x');
    assert_eq!(
        Frame::decode(&raw).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
    // Senders clip on a character boundary instead.
    let clipped = clip_text("ä".repeat(ITEM_TEXT_MAX));
    assert!(clipped.len() <= ITEM_TEXT_MAX && clipped.ends_with('…'));
    assert_eq!(clip_text("kurz".into()), "kurz");
}

#[test]
fn transfer_engine_task_uploads_carry_only_their_free_frames() {
    let session = ServerSession::new(Captured::default().sink());
    assert!(session.route(0, Frame::Credit { bytes: 0 }).is_none());
    let trailer = || Frame::ItemEnd {
        index: 0,
        error: None,
    };

    // A batch of one entry: one trailer plus the final End are free.
    let entries = vec![BatchEntry {
        path: "/x".into(),
        size: 0,
        nonce: 1,
    }];
    let batch = session.open(3, &Frame::BatchPut { entries });
    let inbound = batch.inbound.expect("batch uploads receive frames");
    for _ in 0..2 {
        assert!(session.route(3, trailer()).is_none());
        assert!(!batch.cancel.load(Ordering::Relaxed));
    }
    assert!(session.route(3, trailer()).is_none());
    assert!(batch.cancel.load(Ordering::Relaxed), "a third free frame");
    for _ in 0..2 {
        assert!(inbound.recv_timeout(Duration::from_secs(1)).is_ok());
    }
    assert!(matches!(
        inbound.recv_timeout(Duration::from_secs(1)),
        Err(RecvTimeoutError::Disconnected)
    ));

    // Any other upload has exactly one free frame: its End.
    let write = session.open(4, &Frame::Write("/y".into()));
    assert!(session.route(4, trailer()).is_none());
    assert!(!write.cancel.load(Ordering::Relaxed));
    assert!(session.route(4, trailer()).is_none());
    assert!(write.cancel.load(Ordering::Relaxed));
}

#[test]
fn transfer_engine_task_batch_headers_above_the_file_limit_are_refused() {
    let entry = BatchEntry {
        path: "/a".into(),
        size: 0,
        nonce: 0,
    };
    let full = Frame::BatchPut {
        entries: vec![entry.clone(); BATCH_MAX_FILES],
    };
    assert_eq!(Frame::decode(&full.encode(1).unwrap()).unwrap().1, full);
    let over = Frame::BatchPut {
        entries: vec![entry; BATCH_MAX_FILES + 1],
    };
    assert_eq!(over.encode(1).unwrap_err().kind(), ErrorKind::InvalidData);
    let items = Frame::BatchGet {
        items: vec![
            BatchItem {
                path: "/a".into(),
                id: None,
                size: 0,
            };
            BATCH_MAX_FILES + 1
        ],
    };
    assert_eq!(items.encode(1).unwrap_err().kind(), ErrorKind::InvalidData);

    // A peer that sends more anyway is refused before anything is allocated.
    for (tag, element) in [(35u8, 20usize), (36, 13)] {
        let mut raw = Vec::new();
        raw.extend_from_slice(&1u64.to_le_bytes());
        raw.push(tag);
        raw.extend_from_slice(&((BATCH_MAX_FILES + 1) as u32).to_le_bytes());
        raw.resize(raw.len() + (BATCH_MAX_FILES + 1) * element, 0);
        assert_eq!(
            Frame::decode(&raw).unwrap_err().kind(),
            ErrorKind::InvalidData
        );
    }
}

#[test]
fn transfer_engine_task_only_generated_stage_names_are_discardable() {
    for name in [
        "a.txt.se-upload-0123456789abcdef",
        "a.se-daemon-tree-00000000000000ff",
        "a.bin.se-agent-batch-0000000000000007-0.part",
        "a.bin.se-agent-batch-00000000000000aa-1f.part",
    ] {
        assert!(is_discardable_stage(name), "{name}");
    }
    for name in [
        "notes.txt",
        "a.se-copy-1",
        "a.txt.se-upload-0123456789ABCDEF",
        "a.txt.se-upload-0123456789abcdef.txt",
        ".se-upload-0123456789abcdef",
        "a.se--0123456789abcdef",
        "a.se-Upload-0123456789abcdef",
        "a.se-agent-write-1-2-0.part",
        "a.se-agent-batch-0000000000000007-.part",
        ".se-agent-batch-0000000000000007-0.part",
        "a.se-agent-batch-000000000000007-0.part",
    ] {
        assert!(!is_discardable_stage(name), "{name}");
    }
}

#[cfg(unix)]
#[test]
fn transfer_engine_task_discard_keeps_user_files_and_directories() {
    let root = temp_dir("discard");
    let user = root.join("report.se-copy-1");
    std::fs::write(&user, b"user data").unwrap();
    let error = discard_stage(&user.to_string_lossy()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::PermissionDenied);
    assert_eq!(std::fs::read(&user).unwrap(), b"user data");

    let directory = root.join("dir.se-upload-0123456789abcdef");
    std::fs::create_dir(&directory).unwrap();
    assert!(discard_stage(&directory.to_string_lossy()).is_err());
    assert!(directory.is_dir());

    let stage = root.join("report.txt.se-upload-0123456789abcdef");
    std::fs::write(&stage, b"partial").unwrap();
    discard_stage(&stage.to_string_lossy()).unwrap();
    assert!(!stage.exists());
    let _ = std::fs::remove_dir_all(root);
}
