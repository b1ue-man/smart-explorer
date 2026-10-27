//! `query::FILE_DIGEST` on the agent: a SHA-256 `WalkHashed2` on one regular
//! file answers exactly one root `HashEntry` and `End`; a folder root stays
//! unsupported, a missing file fails, and MD5 walks keep refusing file roots.
use super::ext_ops::{answer, handle_walk_hashed2};
use super::session::Sink;
use super::{digest, query, read_frame, Frame, UNSUPPORTED_EXTENSION};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

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
        "se_file_digest_{label}_{}_{}",
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

/// One walk request on `root`; its result and every frame it sent.
fn walk(root: &std::path::Path, algorithm: u8) -> (io::Result<()>, Vec<Frame>) {
    let captured = Captured::default();
    let result = handle_walk_hashed2(
        &captured.sink(),
        9,
        &root.to_string_lossy(),
        algorithm,
        0,
        &AtomicBool::new(false),
    );
    (result, captured.frames())
}

#[test]
fn screen_metadata_runtime_task_agent_file_walk_returns_one_entry() {
    assert_eq!(query::FILE_DIGEST, 7);
    assert_eq!(answer(query::FILE_DIGEST, "/").unwrap(), Frame::Answer(1));
    let root = temp_dir("one");
    let file = root.join("abc.txt");
    std::fs::write(&file, b"abc").unwrap();
    let (result, frames) = walk(&file, digest::SHA256);
    result.unwrap();
    assert_eq!(frames.len(), 2, "{frames:?}");
    match &frames[0] {
        Frame::HashEntry {
            rel,
            is_dir,
            size,
            md5,
            ..
        } => {
            assert_eq!(rel, "");
            assert!(!is_dir);
            assert_eq!(*size, 3);
            assert_eq!(md5.as_deref(), Some(ABC_SHA256));
        }
        other => panic!("expected the file entry, got {other:?}"),
    }
    assert_eq!(frames[1], Frame::End);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn screen_metadata_runtime_task_agent_file_walk_keeps_other_roots_unchanged() {
    let root = temp_dir("roots");
    let file = root.join("abc.txt");
    std::fs::write(&file, b"abc").unwrap();
    // A folder with SHA-256 stays "unsupported" (the client falls back).
    let (result, frames) = walk(&root, digest::SHA256);
    result.unwrap();
    assert_eq!(frames, [Frame::Err(UNSUPPORTED_EXTENSION.to_string())]);
    // A missing file is an error, never a digest or "unsupported".
    let (result, frames) = walk(&root.join("gone.txt"), digest::SHA256);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotFound);
    assert!(frames.is_empty(), "{frames:?}");
    // MD5 walks still take folders only.
    let (result, _) = walk(&file, digest::MD5);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    let _ = std::fs::remove_dir_all(&root);
}
