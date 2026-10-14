//! Folder deletion in one agent request: the `RemoveTree` frame and its
//! `Query REMOVE_TREE` on the wire, and the agent's handler on a real
//! directory (nested folders, links removed without touching their targets,
//! refused link and file roots, cancel before the next entry, progress).
use super::ext_ops::answer;
use super::session::Sink;
use super::tree_remove::handle_remove_tree;
use super::{query, read_frame, Frame};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

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
        "se_delete_speed_{label}_{}_{}",
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

/// Run the handler on `path`; its result and every frame it sent.
fn remove(path: &Path, canceled: bool) -> (io::Result<()>, Vec<Frame>) {
    let captured = Captured::default();
    let result = handle_remove_tree(
        &captured.sink(),
        5,
        &path.to_string_lossy(),
        &AtomicBool::new(canceled),
    );
    (result, captured.frames())
}

#[test]
fn delete_speed_task_remove_tree_frame_round_trips_with_its_query() {
    assert_eq!(query::REMOVE_TREE, 6);
    let frame = Frame::RemoveTree("/srv/daten/Ordner mit ü".into());
    let encoded = frame.encode(42).unwrap();
    assert_eq!(encoded[8], 62, "tag after the request id");
    assert_eq!(encoded.len(), frame.wire_len().unwrap());
    assert_eq!(Frame::decode(&encoded).unwrap(), (42, frame));

    let question = Frame::Query {
        kind: query::REMOVE_TREE,
        path: "/srv".into(),
    };
    let encoded = question.encode(7).unwrap();
    assert_eq!(Frame::decode(&encoded).unwrap(), (7, question));

    let mut longer = Frame::RemoveTree("/a".into()).encode(1).unwrap();
    longer.push(0);
    assert!(Frame::decode(&longer).is_err());
    let full = Frame::RemoveTree("/abc".into()).encode(1).unwrap();
    assert!(Frame::decode(&full[..full.len() - 1]).is_err());
}

#[test]
fn delete_speed_task_agent_offers_remove_tree_and_keeps_unknown_kinds_at_zero() {
    assert_eq!(answer(query::REMOVE_TREE, "/").unwrap(), Frame::Answer(1));
    // The answer every older server gives for a kind it does not know.
    assert_eq!(answer(200, "/").unwrap(), Frame::Answer(0));
}

#[test]
fn delete_speed_task_agent_removes_a_nested_folder_with_progress() {
    let base = temp_dir("nested");
    let root = base.join("victim");
    std::fs::create_dir_all(root.join("a/b/c")).unwrap();
    std::fs::write(root.join("a/b/c/deep.txt"), b"deep").unwrap();
    std::fs::write(root.join("a/one.txt"), b"1").unwrap();
    std::fs::write(root.join("top.txt"), b"top").unwrap();
    std::fs::create_dir(root.join("empty")).unwrap();
    std::fs::write(base.join("neighbour.txt"), b"stays").unwrap();

    let (result, frames) = remove(&root, false);
    result.unwrap();
    assert!(std::fs::symlink_metadata(&root).is_err());
    assert_eq!(std::fs::read(base.join("neighbour.txt")).unwrap(), b"stays");
    // Three children plus the folder itself.
    assert_eq!(frames.first(), Some(&Frame::Progress { done: 0, total: 4 }));
    assert_eq!(
        &frames[frames.len() - 2..],
        &[Frame::Progress { done: 4, total: 4 }, Frame::End][..]
    );
    assert!(frames[..frames.len() - 1]
        .iter()
        .all(|frame| matches!(frame, Frame::Progress { total: 4, .. })));
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn delete_speed_task_agent_removes_links_without_touching_their_targets() {
    let base = temp_dir("links");
    let outside = base.join("outside");
    std::fs::create_dir_all(outside.join("inner")).unwrap();
    std::fs::write(outside.join("inner/keep.txt"), b"keep").unwrap();
    std::fs::write(base.join("outside.txt"), b"keep too").unwrap();
    let root = base.join("victim");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("dir_link")).unwrap();
    std::os::unix::fs::symlink(base.join("outside.txt"), root.join("file_link")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("sub/nested_link")).unwrap();

    let (result, frames) = remove(&root, false);
    result.unwrap();
    assert!(std::fs::symlink_metadata(&root).is_err());
    assert_eq!(
        std::fs::read(outside.join("inner/keep.txt")).unwrap(),
        b"keep"
    );
    assert_eq!(
        std::fs::read(base.join("outside.txt")).unwrap(),
        b"keep too"
    );
    assert_eq!(frames.last(), Some(&Frame::End));
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn delete_speed_task_agent_refuses_a_file_root() {
    let base = temp_dir("file_root");
    let file = base.join("plain.txt");
    std::fs::write(&file, b"plain").unwrap();
    let (result, frames) = remove(&file, false);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    assert!(frames.is_empty(), "{frames:?}");
    assert_eq!(std::fs::read(&file).unwrap(), b"plain");
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn delete_speed_task_agent_refuses_a_link_root_and_keeps_its_target() {
    let base = temp_dir("link_root");
    let target = base.join("target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("keep.txt"), b"keep").unwrap();
    let link = base.join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let (result, frames) = remove(&link, false);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    assert!(frames.is_empty(), "{frames:?}");
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read(target.join("keep.txt")).unwrap(), b"keep");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn delete_speed_task_agent_cancel_stops_before_the_next_entry() {
    let base = temp_dir("cancel");
    let root = base.join("victim");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("one.txt"), b"1").unwrap();
    std::fs::write(root.join("two.txt"), b"2").unwrap();
    let (result, frames) = remove(&root, true);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
    assert!(root.join("one.txt").exists() && root.join("two.txt").exists());
    assert_eq!(frames, vec![Frame::Progress { done: 0, total: 3 }]);
    let _ = std::fs::remove_dir_all(base);
}
