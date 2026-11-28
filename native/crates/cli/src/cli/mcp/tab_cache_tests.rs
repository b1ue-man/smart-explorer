//! The session's kept connections (plan 2026-11-27 section C): opened once,
//! reused, dropped after a connection failure and reopened exactly once.
use super::{classify, connection_broken, same_backend, Failure, Slots};
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta, VfsResult};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// A remote whose connection can be cut: every call then fails with
/// `ConnectionReset`; `/missing` never exists.
#[derive(Default)]
struct Line {
    cut: AtomicBool,
}

impl Line {
    fn check(&self, path: &str) -> io::Result<()> {
        if self.cut.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::ConnectionReset.into());
        }
        if path == "/missing" {
            return Err(io::ErrorKind::NotFound.into());
        }
        Ok(())
    }
}

impl Backend for Line {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.check(path).map(|()| Vec::new())
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.check(path).map(|()| VfsMeta {
            name: path.rsplit('/').next().unwrap_or_default().to_string(),
            is_dir: true,
            ..Default::default()
        })
    }
    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn io::Read + Send>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn io::Write + Send>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[test]
fn mcp_root_task_connections_are_kept_and_reopened_once_after_a_failure() {
    let slots = Slots::<BackendHandle>::default();
    let opened = AtomicUsize::new(0);
    let lines = std::sync::Mutex::new(Vec::<Arc<Line>>::new());
    let open = || {
        opened.fetch_add(1, Ordering::SeqCst);
        let line = Arc::new(Line::default());
        lines.lock().unwrap().push(line.clone());
        Ok(line as BackendHandle)
    };
    let (first, fresh) = slots.get_or_open("sftp://a@h:22/srv", open).unwrap();
    assert!(fresh);
    // Every later call of the session reuses the open connection.
    for _ in 0..3 {
        let (again, fresh) = slots.get_or_open("sftp://a@h:22/srv", open).unwrap();
        assert!(!fresh);
        assert!(same_backend(&first, &again));
    }
    assert_eq!(opened.load(Ordering::SeqCst), 1);
    // Another key is another connection.
    let (other, _) = slots.get_or_open("share://direct/p/Docs", open).unwrap();
    assert!(!same_backend(&first, &other));
    assert_eq!(opened.load(Ordering::SeqCst), 2);

    // A missing file says nothing about the connection.
    assert!(!connection_broken(&*first, "/missing"));
    // The line is cut: the probe sees it, the connection is dropped.
    lines.lock().unwrap()[0].cut.store(true, Ordering::SeqCst);
    assert!(connection_broken(&*first, "/srv"));
    assert_eq!(slots.evict_where(|kept| same_backend(kept, &first)), 1);
    // The next call opens it again exactly once; the other one stays.
    let (reopened, fresh) = slots.get_or_open("sftp://a@h:22/srv", open).unwrap();
    assert!(fresh);
    assert!(!same_backend(&first, &reopened));
    assert!(reopened.stat("/srv").is_ok());
    let (_, fresh) = slots.get_or_open("sftp://a@h:22/srv", open).unwrap();
    assert!(!fresh);
    let (kept, fresh) = slots.get_or_open("share://direct/p/Docs", open).unwrap();
    assert!(!fresh && same_backend(&kept, &other));
    assert_eq!(opened.load(Ordering::SeqCst), 3);

    // A failed open keeps nothing: the next call tries once more.
    let error = slots
        .get_or_open("@down:/", || Err("unreachable".to_string()))
        .err()
        .unwrap();
    assert_eq!(error, "unreachable");
    let (_, fresh) = slots.get_or_open("@down:/", open).unwrap();
    assert!(fresh);
    slots.evict("@down:/");
    let (_, fresh) = slots.get_or_open("@down:/", open).unwrap();
    assert!(fresh);
}

#[test]
fn mcp_root_task_failures_name_the_connection_or_the_entry() {
    for kind in [
        io::ErrorKind::ConnectionReset,
        io::ErrorKind::BrokenPipe,
        io::ErrorKind::NotConnected,
        io::ErrorKind::TimedOut,
        io::ErrorKind::HostUnreachable,
        io::ErrorKind::UnexpectedEof,
    ] {
        assert_eq!(classify(kind), Failure::Connection, "{kind:?}");
    }
    for kind in [
        io::ErrorKind::NotFound,
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::AlreadyExists,
        io::ErrorKind::IsADirectory,
        io::ErrorKind::StorageFull,
    ] {
        assert_eq!(classify(kind), Failure::Entry, "{kind:?}");
    }
    assert_eq!(classify(io::ErrorKind::Other), Failure::Unknown);
}
