use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use super::promotion::{
    ensure_plain_directory_tree, validate_destination_root, validate_file_destination,
    StagedLocalFile, StagingArea,
};
use super::session::{emit, Sink};
use super::{Frame, ValidatedRelativePath};

pub(crate) const MAX_TREE_ENTRIES: u64 = 1_000_000;
pub(crate) const MAX_TREE_TEXT_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) const MAX_TREE_DEPTH: usize = 512;

#[derive(Default)]
pub(crate) struct TreeManifestValidator {
    seen: BTreeMap<String, bool>,
    entries: u64,
    text_bytes: u64,
}

impl TreeManifestValidator {
    pub(crate) fn record(
        &mut self,
        relative: &ValidatedRelativePath,
        is_dir: bool,
    ) -> io::Result<()> {
        let path = relative.as_str();
        if relative.depth() > MAX_TREE_DEPTH {
            return Err(invalid("tree manifest exceeds its depth limit"));
        }
        self.entries = self
            .entries
            .checked_add(1)
            .ok_or_else(|| invalid("tree entry count overflow"))?;
        self.text_bytes = self
            .text_bytes
            .checked_add(path.len() as u64)
            .ok_or_else(|| invalid("tree path text budget overflow"))?;
        if self.entries > MAX_TREE_ENTRIES || self.text_bytes > MAX_TREE_TEXT_BYTES {
            return Err(invalid(
                "tree manifest exceeds its bounded collection budget",
            ));
        }
        if self.seen.contains_key(path) {
            return Err(invalid("tree manifest contains a duplicate relative path"));
        }
        for (index, byte) in path.bytes().enumerate() {
            if byte == b'/' && self.seen.get(&path[..index]) == Some(&false) {
                return Err(invalid("tree manifest places an entry below a file"));
            }
        }
        if !is_dir {
            let descendant_prefix = format!("{path}/");
            if self
                .seen
                .range(descendant_prefix.clone()..)
                .next()
                .is_some_and(|(existing, _)| existing.starts_with(&descendant_prefix))
            {
                return Err(invalid(
                    "tree manifest replaces an entry ancestor with a file",
                ));
            }
        }
        self.seen.insert(path.to_string(), is_dir);
        Ok(())
    }
}

pub(crate) struct BufferedTreeEntry {
    pub(crate) relative: ValidatedRelativePath,
    pub(crate) is_dir: bool,
    pub(crate) file: Option<StagedLocalFile>,
}

struct PendingFile {
    relative: ValidatedRelativePath,
    file: StagedLocalFile,
}

/// Buffers a complete tree stream into a private flat spool. No destination
/// operation is possible until `Frame::End` and every manifest entry validates.
pub(crate) struct BufferedTreeReceiver {
    entries: Vec<BufferedTreeEntry>,
    pending: Option<PendingFile>,
    validator: TreeManifestValidator,
    ended: bool,
    next_file: u64,
    staging: StagingArea,
}

impl BufferedTreeReceiver {
    pub(crate) fn create(purpose: &str, request_id: u64) -> io::Result<Self> {
        Ok(Self {
            entries: Vec::new(),
            pending: None,
            validator: TreeManifestValidator::default(),
            ended: false,
            next_file: 0,
            staging: StagingArea::create(purpose, request_id)?,
        })
    }

    /// Returns true only after accepting the one terminal `End` frame.
    pub(crate) fn accept(&mut self, frame: Frame) -> io::Result<bool> {
        if self.ended {
            return Err(invalid("tree stream continued after its end frame"));
        }
        match frame {
            Frame::TreeEntry {
                rel, is_dir, size, ..
            } => {
                self.finish_pending()?;
                let relative = ValidatedRelativePath::parse(&rel)?;
                self.validator.record(&relative, is_dir)?;
                if is_dir {
                    self.entries.push(BufferedTreeEntry {
                        relative,
                        is_dir: true,
                        file: None,
                    });
                } else {
                    let file = self.staging.create_file(self.next_file, size)?;
                    self.next_file += 1;
                    self.pending = Some(PendingFile { relative, file });
                }
                Ok(false)
            }
            Frame::Data(data) => {
                self.pending
                    .as_mut()
                    .ok_or_else(|| invalid("tree data arrived without a file entry"))?
                    .file
                    .write_chunk(&data)?;
                Ok(false)
            }
            Frame::End => {
                self.finish_pending()?;
                self.ended = true;
                Ok(true)
            }
            _ => Err(invalid("unexpected frame in tree stream")),
        }
    }

    fn finish_pending(&mut self) -> io::Result<()> {
        if let Some(mut pending) = self.pending.take() {
            pending.file.finish()?;
            self.entries.push(BufferedTreeEntry {
                relative: pending.relative,
                is_dir: false,
                file: Some(pending.file),
            });
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> io::Result<BufferedTree> {
        if !self.ended || self.pending.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "tree stream ended without a terminal frame",
            ));
        }
        Ok(BufferedTree {
            entries: self.entries,
            file_count: self.next_file,
            _staging: self.staging,
        })
    }
}

pub(crate) struct BufferedTree {
    pub(crate) entries: Vec<BufferedTreeEntry>,
    file_count: u64,
    // Kept last so entry files are dropped before their containing directory.
    _staging: StagingArea,
}

impl BufferedTree {
    pub(crate) fn file_count(&self) -> u64 {
        self.file_count
    }

    pub(crate) fn publish_local(
        mut self,
        root: &Path,
        purpose: &str,
        request_id: u64,
    ) -> io::Result<u64> {
        validate_destination_root(root)?;
        // Inspect every existing destination ancestor before the first mkdir or
        // publication. The apply phase repeats these checks to close races.
        for entry in &self.entries {
            let destination = entry.relative.join_local(root);
            if entry.is_dir {
                validate_destination_root(&destination)?;
            } else {
                validate_file_destination(&destination)?;
            }
        }
        ensure_plain_directory_tree(root)?;
        for entry in self.entries.iter().filter(|entry| entry.is_dir) {
            ensure_plain_directory_tree(&entry.relative.join_local(root))?;
        }
        for entry in self.entries.iter_mut().filter(|entry| !entry.is_dir) {
            let destination = entry.relative.join_local(root);
            entry
                .file
                .take()
                .ok_or_else(|| io::Error::other("tree file has no buffered content"))?
                .publish_local(&destination, purpose, request_id)?;
        }
        Ok(self.file_count)
    }
}

fn canceled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "put-tree canceled")
}

/// Receive and validate an entire subtree before making the first destination
/// mutation, then publish each complete file through an atomic staged replace.
pub(crate) fn handle_put_tree(
    sink: &Sink,
    id: u64,
    root: &str,
    inbound: &Receiver<Frame>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let root = Path::new(root);
    validate_destination_root(root)?;
    let mut receiver = BufferedTreeReceiver::create("tree", id)?;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled());
        }
        let frame = match inbound.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => frame,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "put-tree aborted",
                ));
            }
        };
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled());
        }
        if receiver.accept(frame)? {
            break;
        }
    }
    receiver.finish()?.publish_local(root, "tree", id)?;
    emit(sink, id, &Frame::Ok)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::{handle_put_tree, TreeManifestValidator, MAX_TREE_ENTRIES, MAX_TREE_TEXT_BYTES};
    use crate::agent_proto::session::Sink;
    use crate::agent_proto::{Frame, ValidatedRelativePath};
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex};

    fn test_root(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "se_agent_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn sink() -> Sink {
        Arc::new(Mutex::new(Box::new(Vec::<u8>::new())))
    }

    fn run(root: &std::path::Path, frames: Vec<Frame>) -> std::io::Result<()> {
        let (tx, rx) = channel();
        for frame in frames {
            tx.send(frame).unwrap();
        }
        drop(tx);
        handle_put_tree(
            &sink(),
            1,
            &root.to_string_lossy(),
            &rx,
            &AtomicBool::new(false),
        )
    }

    #[test]
    fn invalid_manifest_never_creates_the_destination_root() {
        for (label, frames) in [
            (
                "duplicate",
                vec![
                    Frame::TreeEntry {
                        rel: "same".into(),
                        is_dir: false,
                        size: 0,
                        mtime_ms: 0,
                    },
                    Frame::TreeEntry {
                        rel: "same".into(),
                        is_dir: false,
                        size: 0,
                        mtime_ms: 0,
                    },
                ],
            ),
            (
                "conflict",
                vec![
                    Frame::TreeEntry {
                        rel: "parent".into(),
                        is_dir: false,
                        size: 0,
                        mtime_ms: 0,
                    },
                    Frame::TreeEntry {
                        rel: "parent/child".into(),
                        is_dir: true,
                        size: 0,
                        mtime_ms: 0,
                    },
                ],
            ),
            (
                "backslash",
                vec![Frame::TreeEntry {
                    rel: r"literal\name".into(),
                    is_dir: false,
                    size: 0,
                    mtime_ms: 0,
                }],
            ),
        ] {
            let root = test_root(label);
            let _ = std::fs::remove_dir_all(&root);
            assert_eq!(
                run(&root, frames).unwrap_err().kind(),
                std::io::ErrorKind::InvalidData
            );
            assert!(!root.exists());
        }
    }

    #[test]
    fn disconnect_preserves_existing_destination_and_does_not_create_missing_root() {
        let root = test_root("disconnect");
        std::fs::create_dir_all(&root).unwrap();
        let destination = root.join("file.txt");
        std::fs::write(&destination, b"old").unwrap();
        let frame = Frame::TreeEntry {
            rel: "file.txt".into(),
            is_dir: false,
            size: 3,
            mtime_ms: 0,
        };
        assert_eq!(
            run(&root, vec![frame]).unwrap_err().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"old");

        let missing = test_root("missing_disconnect");
        let _ = std::fs::remove_dir_all(&missing);
        let frame = Frame::TreeEntry {
            rel: "file.txt".into(),
            is_dir: false,
            size: 0,
            mtime_ms: 0,
        };
        assert!(run(&missing, vec![frame]).is_err());
        assert!(!missing.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn manifest_budget_and_depth_are_bounded_without_large_allocations() {
        let mut validator = TreeManifestValidator {
            entries: MAX_TREE_ENTRIES,
            ..Default::default()
        };
        assert!(validator
            .record(&ValidatedRelativePath::parse("next").unwrap(), false)
            .is_err());

        let mut validator = TreeManifestValidator {
            text_bytes: MAX_TREE_TEXT_BYTES,
            ..Default::default()
        };
        assert!(validator
            .record(&ValidatedRelativePath::parse("next").unwrap(), false)
            .is_err());

        let deep = std::iter::repeat_n("a", 513).collect::<Vec<_>>().join("/");
        assert!(TreeManifestValidator::default()
            .record(&ValidatedRelativePath::parse(&deep).unwrap(), true)
            .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn link_like_destination_root_and_ancestor_are_rejected() {
        use std::os::unix::fs::symlink;

        let base = test_root("links");
        let victim = base.join("victim");
        let root_link = base.join("root-link");
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::write(victim.join("sentinel"), b"keep").unwrap();
        symlink(&victim, &root_link).unwrap();
        assert!(run(&root_link, vec![Frame::End]).is_err());

        let ancestor_link = base.join("ancestor-link");
        symlink(&victim, &ancestor_link).unwrap();
        assert!(run(&ancestor_link.join("new-root"), vec![Frame::End]).is_err());
        assert_eq!(std::fs::read(victim.join("sentinel")).unwrap(), b"keep");
        assert!(!victim.join("new-root").exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[test]
    fn later_link_like_destination_fails_before_any_file_is_published() {
        use std::os::unix::fs::symlink;

        let base = test_root("late_link");
        let root = base.join("root");
        let victim = base.join("victim");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&victim).unwrap();
        symlink(&victim, root.join("link")).unwrap();
        let frames = vec![
            Frame::TreeEntry {
                rel: "safe.txt".into(),
                is_dir: false,
                size: 3,
                mtime_ms: 0,
            },
            Frame::Data(b"new".to_vec()),
            Frame::TreeEntry {
                rel: "link/escape.txt".into(),
                is_dir: false,
                size: 0,
                mtime_ms: 0,
            },
            Frame::End,
        ];
        assert!(run(&root, frames).is_err());
        assert!(!root.join("safe.txt").exists());
        assert!(!victim.join("escape.txt").exists());
        let _ = std::fs::remove_dir_all(base);
    }
}
