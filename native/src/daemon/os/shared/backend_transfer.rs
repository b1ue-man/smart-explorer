use std::borrow::Cow;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use crate::agent_proto::{BufferedTree, BufferedTreeReceiver, Frame};
use crate::vfs::BackendHandle;

use super::backend_server::{emit, Sink};

fn join_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else if parent == "/" {
        format!("/{name}")
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), name)
    }
}

pub(super) fn handle_put_tree_backend(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    inbound: &Receiver<Frame>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let root = canonical_backend_root(backend, root);
    let root = root.as_ref();
    validate_backend_destination(backend, root)?;
    let mut receiver = BufferedTreeReceiver::create("daemon-tree", id)?;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "daemon backend put-tree canceled",
            ));
        }
        let frame = match inbound.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => frame,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "daemon backend put-tree aborted",
                ));
            }
        };
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "daemon backend put-tree canceled",
            ));
        }
        if receiver.accept(frame)? {
            break;
        }
    }
    publish_backend_tree(backend, root, receiver.finish()?)?;
    emit(sink, id, &Frame::Ok)
}

pub(super) fn canonical_backend_root<'a>(backend: &BackendHandle, root: &'a str) -> Cow<'a, str> {
    if backend.is_local() {
        super::platform::normalize_local_backend_path(root)
    } else {
        Cow::Borrowed(root)
    }
}

fn publish_backend_tree(
    backend: &BackendHandle,
    root: &str,
    mut tree: BufferedTree,
) -> io::Result<u64> {
    validate_backend_destination(backend, root)?;
    for entry in &tree.entries {
        let destination = join_path(root, entry.relative.as_str());
        if entry.is_dir {
            validate_backend_destination(backend, &destination)?;
        } else {
            let parent = destination
                .rsplit_once('/')
                .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "daemon tree file destination has no parent",
                    )
                })?;
            validate_backend_destination(backend, parent)?;
            validate_backend_file_destination(backend, &destination)?;
        }
    }
    backend.mkdir_all(root)?;
    require_plain_backend_directory(backend, root)?;
    for entry in tree.entries.iter().filter(|entry| entry.is_dir) {
        let destination = join_path(root, entry.relative.as_str());
        validate_backend_destination(backend, &destination)?;
        backend.mkdir_all(&destination)?;
        require_plain_backend_directory(backend, &destination)?;
    }
    for entry in tree.entries.iter_mut().filter(|entry| !entry.is_dir) {
        let destination = join_path(root, entry.relative.as_str());
        if let Some(parent) = destination.rsplit_once('/').map(|(parent, _)| parent) {
            if !parent.is_empty() {
                validate_backend_destination(backend, parent)?;
                backend.mkdir_all(parent)?;
                require_plain_backend_directory(backend, parent)?;
            }
        }
        validate_backend_file_destination(backend, &destination)?;
        let staged = crate::vfs::unique_staging_path(&**backend, &destination, "daemon-tree")?;
        let result = (|| {
            let mut writer = backend.open_write(&staged)?;
            entry
                .file
                .as_ref()
                .ok_or_else(|| io::Error::other("daemon tree file has no buffered content"))?
                .copy_to_writer(&mut writer)?;
            writer.flush()?;
            drop(writer);
            backend.promote_staged(&staged, &destination)
        })();
        if result.is_err() {
            let _ = backend.remove_file(&staged);
        }
        result?;
        drop(entry.file.take());
    }
    Ok(tree.file_count())
}

pub(super) fn validate_backend_destination(backend: &BackendHandle, path: &str) -> io::Result<()> {
    for ancestor in backend_ancestors(path)? {
        match backend.stat(&ancestor) {
            Ok(metadata) if metadata.is_symlink => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("daemon tree destination ancestor is link-like: {ancestor}"),
                ));
            }
            Ok(metadata) if !metadata.is_dir => {
                return Err(io::Error::new(
                    io::ErrorKind::NotADirectory,
                    format!("daemon tree destination ancestor is not a directory: {ancestor}"),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn require_plain_backend_directory(backend: &BackendHandle, path: &str) -> io::Result<()> {
    let metadata = backend.stat(path)?;
    if metadata.is_symlink || !metadata.is_dir {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("daemon tree destination is not a plain directory: {path}"),
        ));
    }
    Ok(())
}

fn validate_backend_file_destination(backend: &BackendHandle, path: &str) -> io::Result<()> {
    match backend.stat(path) {
        Ok(metadata) if metadata.is_dir || metadata.is_symlink => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "daemon tree file destination is a directory or link-like entry",
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn backend_ancestors(path: &str) -> io::Result<Vec<String>> {
    let unc = path.starts_with("//") && !path.starts_with("///");
    let absolute = path.starts_with('/') && !unc;
    let bytes = path.as_bytes();
    let drive_prefixed =
        bytes.get(1) == Some(&b':') && bytes.first().is_some_and(u8::is_ascii_alphabetic);
    let drive_absolute = drive_prefixed && bytes.get(2) == Some(&b'/');
    if drive_prefixed && !drive_absolute {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "daemon tree destination uses a drive-relative path",
        ));
    }
    let mut current = if absolute { "/" } else { "" }.to_string();
    let mut ancestors = if absolute {
        vec!["/".to_string()]
    } else {
        Vec::new()
    };
    for (index, component) in path
        .split('/')
        .filter(|component| !component.is_empty())
        .enumerate()
    {
        if matches!(component, "." | "..") || component.contains('\\') || component.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "daemon tree destination contains an unsafe path component",
            ));
        }
        if drive_absolute && index == 0 {
            current = format!("{component}/");
        } else if unc && index == 0 {
            current = format!("//{component}");
            continue;
        } else {
            current = join_path(&current, component);
        }
        ancestors.push(current.clone());
    }
    if ancestors.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "daemon tree destination root is empty",
        ));
    }
    Ok(ancestors)
}

#[cfg(test)]
mod tests {
    use super::{backend_ancestors, handle_put_tree_backend, Sink};
    use crate::agent_proto::Frame;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn test_root(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "se_daemon_{label}_{}_{}",
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

    fn local_backend() -> crate::vfs::BackendHandle {
        Arc::new(crate::vfs::LocalBackend::new("/"))
    }

    fn has_stage(root: &std::path::Path) -> bool {
        std::fs::read_dir(root).unwrap().flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .contains(".se-daemon-tree-")
        })
    }

    #[test]
    fn destination_ancestors_accept_canonical_windows_absolute_paths() {
        assert_eq!(
            backend_ancestors("C:/Users/Alice/transfer").unwrap(),
            [
                "C:/",
                "C:/Users",
                "C:/Users/Alice",
                "C:/Users/Alice/transfer"
            ]
        );
        assert_eq!(
            backend_ancestors("//server/share/transfer").unwrap(),
            ["//server/share", "//server/share/transfer"]
        );
    }

    #[test]
    fn destination_ancestors_keep_rejecting_ambiguous_backslashes() {
        assert!(backend_ancestors(r"C:relative\file").is_err());
        assert!(backend_ancestors(r"C:\absolute\file").is_err());
        assert!(backend_ancestors(r"\\server\share\file").is_err());
        assert!(backend_ancestors(r"/unix/name\with-backslash").is_err());
        assert!(backend_ancestors("C:/safe/../escape").is_err());
    }

    #[test]
    fn put_tree_rejects_parent_path_before_opening_destination() {
        let base = std::env::temp_dir().join(format!(
            "se_daemon_bulk_escape_{}_{}",
            std::process::id(),
            crate::share::core_now_secs()
        ));
        let root = base.join("root");
        let escaped = base.join("escaped.txt");
        let _ = std::fs::remove_dir_all(&base);

        let backend: crate::vfs::BackendHandle =
            Arc::new(crate::vfs::LocalBackend::new(&base.to_string_lossy()));
        let sink: Sink = Arc::new(Mutex::new(Box::new(Vec::<u8>::new())));
        let (tx, rx) = channel();
        tx.send(Frame::TreeEntry {
            rel: "../escaped.txt".into(),
            is_dir: false,
            size: 1,
            mtime_ms: 0,
        })
        .unwrap();
        tx.send(Frame::Data(vec![1])).unwrap();
        tx.send(Frame::End).unwrap();

        let error = handle_put_tree_backend(
            &sink,
            1,
            &backend,
            &root.to_string_lossy(),
            &rx,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(!escaped.exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn disconnect_preserves_existing_destination_and_removes_stage() {
        let root = test_root("bulk_disconnect");
        std::fs::create_dir_all(&root).unwrap();
        let destination = root.join("file.txt");
        std::fs::write(&destination, b"old").unwrap();
        let backend = local_backend();
        let (tx, rx) = channel();
        tx.send(Frame::TreeEntry {
            rel: "file.txt".into(),
            is_dir: false,
            size: 3,
            mtime_ms: 0,
        })
        .unwrap();
        tx.send(Frame::Data(b"new".to_vec())).unwrap();
        drop(tx);

        let error = handle_put_tree_backend(
            &sink(),
            2,
            &backend,
            &root.to_string_lossy(),
            &rx,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
        assert_eq!(std::fs::read(&destination).unwrap(), b"old");
        assert!(!has_stage(&root));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cancellation_preserves_existing_destination_and_removes_stage() {
        let root = test_root("bulk_cancel");
        std::fs::create_dir_all(&root).unwrap();
        let destination = root.join("file.txt");
        std::fs::write(&destination, b"old").unwrap();
        let backend = local_backend();
        let (tx, rx) = channel();
        tx.send(Frame::TreeEntry {
            rel: "file.txt".into(),
            is_dir: false,
            size: 8,
            mtime_ms: 0,
        })
        .unwrap();
        tx.send(Frame::Data(b"partial".to_vec())).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker_root = root.clone();
        let worker_backend = backend.clone();
        let worker = std::thread::spawn(move || {
            handle_put_tree_backend(
                &sink(),
                3,
                &worker_backend,
                &worker_root.to_string_lossy(),
                &rx,
                &worker_cancel,
            )
        });
        for _ in 0..100 {
            if has_stage(&root) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        cancel.store(true, Ordering::Relaxed);
        let error = worker.join().unwrap().unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
        assert_eq!(std::fs::read(&destination).unwrap(), b"old");
        assert!(!has_stage(&root));
        drop(tx);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn put_tree_rejects_link_like_destination_root_and_ancestor() {
        use std::os::unix::fs::symlink;

        let base = test_root("bulk_link_root");
        let victim = base.join("victim");
        let root_link = base.join("root-link");
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::write(victim.join("sentinel"), b"keep").unwrap();
        symlink(&victim, &root_link).unwrap();
        let backend = local_backend();

        for destination in [root_link.clone(), root_link.join("nested")] {
            let (tx, rx) = channel();
            tx.send(Frame::End).unwrap();
            assert!(handle_put_tree_backend(
                &sink(),
                4,
                &backend,
                &destination.to_string_lossy(),
                &rx,
                &AtomicBool::new(false),
            )
            .is_err());
        }
        assert_eq!(std::fs::read(victim.join("sentinel")).unwrap(), b"keep");
        assert!(!victim.join("nested").exists());
        let _ = std::fs::remove_dir_all(base);
    }
}
