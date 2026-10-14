//! The background service's `RemoveTree`: one frame from the client runs
//! the served backend's own folder deletion, its counters come back as
//! progress, a failure keeps the counters reached, a cancel reaches the
//! backend's run, and a backend without the extension answers 0 and refuses
//! the frame without deleting anything.
use super::super::backend_server::serve_backend;
use crate::agent::AgentBackend;
use crate::agent_proto::{query, read_frame, write_frame, Frame, UNSUPPORTED_EXTENSION};
use crate::vfs::{
    self as vfs, Backend, BackendExtensions, BackendHandle, DeleteTarget, RecursiveDeleteFailure,
    RecursiveDeletePhase, RecursiveDeleteProgress, RecursiveDeleteReport, RecursiveDeleteStatus,
    Scheme, VfsMeta, VfsResult,
};
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
enum Mode {
    Complete,
    Fail,
    WaitForCancel,
}

/// A peer stand-in whose host deletes whole folders.
struct TreeBackend {
    mode: Mode,
    calls: Arc<Mutex<Vec<String>>>,
    saw_cancel: Arc<AtomicBool>,
}

fn unused<T>() -> VfsResult<T> {
    Err(io::Error::other("unused by this test"))
}

fn step(path: &str, planned: u64, removed: u64) -> RecursiveDeleteProgress {
    RecursiveDeleteProgress {
        phase: RecursiveDeletePhase::Applying,
        planned,
        removed,
        current: path.into(),
    }
}

impl Backend for TreeBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
        unused()
    }

    fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
        unused()
    }

    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        unused()
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        unused()
    }

    fn rename(&self, _source: &str, _destination: &str) -> VfsResult<()> {
        unused()
    }

    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        unused()
    }

    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        unused()
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        unused()
    }

    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }
}

impl BackendExtensions for TreeBackend {
    fn supports_remove_tree(&self, _path: &str) -> VfsResult<bool> {
        Ok(true)
    }

    fn remove_tree(
        &self,
        path: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(RecursiveDeleteProgress),
    ) -> Result<RecursiveDeleteReport, RecursiveDeleteFailure> {
        self.calls.lock().unwrap().push(path.into());
        match self.mode {
            Mode::Complete => {
                progress(step(path, 3, 1));
                progress(step(path, 3, 3));
                Ok(RecursiveDeleteReport {
                    status: RecursiveDeleteStatus::Complete,
                    planned: 3,
                    removed: 3,
                })
            }
            Mode::Fail => {
                progress(step(path, 4, 1));
                Err(RecursiveDeleteFailure {
                    error: io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "permission denied by the peer host",
                    ),
                    planned: 4,
                    removed: 1,
                })
            }
            Mode::WaitForCancel => {
                progress(step(path, 9, 1));
                let deadline = Instant::now() + Duration::from_secs(10);
                while !cancel.load(Ordering::Acquire) {
                    if Instant::now() >= deadline {
                        return Err(RecursiveDeleteFailure {
                            error: io::Error::new(io::ErrorKind::TimedOut, "no cancel arrived"),
                            planned: 9,
                            removed: 1,
                        });
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                self.saw_cancel.store(true, Ordering::Release);
                Ok(RecursiveDeleteReport {
                    status: RecursiveDeleteStatus::Canceled,
                    planned: 9,
                    removed: 1,
                })
            }
        }
    }
}

struct Served {
    client: AgentBackend,
    shutdown: TcpStream,
    server: JoinHandle<()>,
}

impl Served {
    fn finish(self) {
        drop(self.client);
        let _ = self.shutdown.shutdown(Shutdown::Both);
        self.server.join().unwrap();
    }
}

/// The service serving `backend`, reached by the GUI's `AgentBackend`.
fn served(backend: BackendHandle) -> Served {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        let _ = serve_backend(read, socket, backend);
    });
    let stream = TcpStream::connect(address).unwrap();
    let shutdown = stream.try_clone().unwrap();
    let read: Box<dyn Read + Send> = Box::new(stream.try_clone().unwrap());
    let write: Box<dyn Write + Send> = Box::new(stream);
    let inner: BackendHandle = Arc::new(vfs::LocalBackend::new("/"));
    Served {
        client: AgentBackend::from_streams(read, write, inner).unwrap(),
        shutdown,
        server,
    }
}

fn tree_backend(mode: Mode) -> (BackendHandle, Arc<Mutex<Vec<String>>>, Arc<AtomicBool>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let saw_cancel = Arc::new(AtomicBool::new(false));
    let backend: BackendHandle = Arc::new(TreeBackend {
        mode,
        calls: calls.clone(),
        saw_cancel: saw_cancel.clone(),
    });
    (backend, calls, saw_cancel)
}

#[test]
fn delete_speed_task_service_runs_the_backend_tree_delete_for_one_frame() {
    let (backend, calls, _) = tree_backend(Mode::Complete);
    let served = served(backend);
    assert!(vfs::supports_remove_tree(&served.client, "/peer/ordner").unwrap());
    let mut reports = Vec::new();
    let report = vfs::remove_entry_controlled(
        &served.client,
        &DeleteTarget {
            path: "/peer/ordner".into(),
            id: None,
            is_dir: true,
            is_symlink: false,
        },
        &AtomicBool::new(false),
        |progress| reports.push(progress),
    )
    .unwrap();
    assert_eq!(
        report,
        RecursiveDeleteReport {
            status: RecursiveDeleteStatus::Complete,
            planned: 3,
            removed: 3,
        }
    );
    assert_eq!(reports.last(), Some(&step("/peer/ordner", 3, 3)));
    assert_eq!(*calls.lock().unwrap(), vec!["/peer/ordner".to_string()]);
    served.finish();
}

#[test]
fn delete_speed_task_service_failure_reports_the_counters_reached() {
    let (backend, calls, _) = tree_backend(Mode::Fail);
    let served = served(backend);
    let failure = vfs::remove_tree(
        &served.client,
        "/peer/locked",
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!((failure.planned, failure.removed), (4, 1));
    assert_eq!(failure.error.kind(), io::ErrorKind::PermissionDenied);
    assert!(failure.error.to_string().contains("peer host"));
    assert_eq!(calls.lock().unwrap().len(), 1);
    served.finish();
}

#[test]
fn delete_speed_task_service_cancel_reaches_the_backend_run() {
    let (backend, _, saw_cancel) = tree_backend(Mode::WaitForCancel);
    let served = served(backend);
    let cancel = AtomicBool::new(false);
    let report = vfs::remove_tree(&served.client, "/peer/big", &cancel, &mut |_| {
        cancel.store(true, Ordering::Release);
    })
    .unwrap();
    assert_eq!(report.status, RecursiveDeleteStatus::Canceled);
    assert_eq!((report.planned, report.removed), (9, 1));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !saw_cancel.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(saw_cancel.load(Ordering::Acquire));
    served.finish();
}

#[test]
fn delete_speed_task_service_without_backend_support_answers_zero_and_refuses_the_frame() {
    let base = std::env::temp_dir().join(format!(
        "se_delete_speed_service_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(base.join("victim")).unwrap();
    std::fs::write(base.join("victim/keep.txt"), b"keep").unwrap();
    let path = base.join("victim").to_string_lossy().into_owned();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        let backend: BackendHandle = Arc::new(vfs::LocalBackend::new("/"));
        let _ = serve_backend(read, socket, backend);
    });
    let mut client = TcpStream::connect(address).unwrap();
    let question = Frame::Query {
        kind: query::REMOVE_TREE,
        path: path.clone(),
    };
    write_frame(&mut client, 3, &question).unwrap();
    assert_eq!(
        read_frame(&mut client).unwrap(),
        Some((3, Frame::Answer(0)))
    );
    write_frame(&mut client, 4, &Frame::RemoveTree(path)).unwrap();
    assert_eq!(
        read_frame(&mut client).unwrap(),
        Some((4, Frame::Err(UNSUPPORTED_EXTENSION.to_string())))
    );
    assert_eq!(
        std::fs::read(base.join("victim/keep.txt")).unwrap(),
        b"keep"
    );
    client.shutdown(Shutdown::Both).unwrap();
    drop(client);
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(base);
}
