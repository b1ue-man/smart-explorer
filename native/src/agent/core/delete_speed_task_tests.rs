//! The agent client's folder deletion (`RemoveTree`): one request to a
//! server that answered `Query REMOVE_TREE` with 1, never the frame to a
//! server that answered 0, the SFTP fallback of an SSH location, cancel and
//! failure counters, and the real agent server deleting a folder.
use super::AgentBackend;
use crate::agent_proto::{query, read_frame, write_frame, Frame, PROTO_VERSION};
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

/// How a scripted server answers.
struct Script {
    version: &'static str,
    answer: u64,
    on_remove: Vec<Frame>,
}

type Seen = Arc<Mutex<Vec<(u64, Frame)>>>;

struct Scripted {
    backend: AgentBackend,
    shutdown: TcpStream,
    seen: Seen,
    server: JoinHandle<()>,
}

impl Scripted {
    /// Wait until the server read a frame matching `wanted`.
    fn wait_for(&self, wanted: impl Fn(u64, &Frame) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if self
                .seen
                .lock()
                .unwrap()
                .iter()
                .any(|(id, frame)| wanted(*id, frame))
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// Every request frame the server read, in order.
    fn finish(self) -> Vec<(u64, Frame)> {
        drop(self.backend);
        let _ = self.shutdown.shutdown(Shutdown::Both);
        self.server.join().unwrap();
        self.seen.lock().unwrap().clone()
    }
}

/// A server that answers the handshake, every `Query` with `answer` and a
/// `RemoveTree` with `on_remove`, and records every frame it reads.
fn scripted(script: Script, inner: BackendHandle) -> Scripted {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let seen: Seen = Arc::default();
    let recorded = seen.clone();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut read = socket.try_clone().unwrap();
        while let Ok(Some((id, frame))) = read_frame(&mut read) {
            let replies = match &frame {
                Frame::Hello { .. } => vec![Frame::HelloOk {
                    proto: PROTO_VERSION,
                    version: script.version.into(),
                }],
                Frame::Query { .. } => vec![Frame::Answer(script.answer)],
                Frame::RemoveTree(_) => script.on_remove.clone(),
                _ => Vec::new(),
            };
            recorded.lock().unwrap().push((id, frame));
            for reply in replies {
                if write_frame(&mut socket, id, &reply).is_err() {
                    return;
                }
            }
        }
    });
    let client = TcpStream::connect(address).unwrap();
    let shutdown = client.try_clone().unwrap();
    let read: Box<dyn Read + Send> = Box::new(client.try_clone().unwrap());
    let write: Box<dyn Write + Send> = Box::new(client);
    Scripted {
        backend: AgentBackend::from_streams(read, write, inner).unwrap(),
        shutdown,
        seen,
        server,
    }
}

fn local_inner() -> BackendHandle {
    Arc::new(vfs::LocalBackend::new("/"))
}

fn folder(path: &str) -> DeleteTarget {
    DeleteTarget {
        path: path.into(),
        id: None,
        is_dir: true,
        is_symlink: false,
    }
}

fn is_remove_tree(frame: &Frame) -> bool {
    matches!(frame, Frame::RemoveTree(_))
}

#[test]
fn delete_speed_task_service_folder_delete_is_one_request() {
    let scripted = scripted(
        Script {
            version: "0.5.0+ext-v1 worker",
            answer: 1,
            on_remove: vec![
                Frame::Progress { done: 0, total: 3 },
                Frame::Progress { done: 3, total: 3 },
                Frame::End,
            ],
        },
        local_inner(),
    );
    let mut reports = Vec::new();
    let report = vfs::remove_entry_controlled(
        &scripted.backend,
        &folder("/srv/share/ordner"),
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
    assert_eq!(
        reports.last(),
        Some(&RecursiveDeleteProgress {
            phase: RecursiveDeletePhase::Applying,
            planned: 3,
            removed: 3,
            current: "/srv/share/ordner".into(),
        })
    );
    let seen = scripted.finish();
    let requests: Vec<&Frame> = seen
        .iter()
        .map(|(_, frame)| frame)
        .filter(|frame| !matches!(frame, Frame::Hello { .. }))
        .collect();
    // Two questions (support, then the same decision) and one deletion
    // request; no listing, metadata or per-entry removal.
    assert_eq!(requests.len(), 3, "{requests:?}");
    assert!(requests[..2].iter().all(|frame| matches!(
        frame,
        Frame::Query { kind, path } if *kind == query::REMOVE_TREE && path == "/srv/share/ordner"
    )));
    assert_eq!(requests[2], &Frame::RemoveTree("/srv/share/ordner".into()));
}

#[test]
fn delete_speed_task_server_answering_zero_never_gets_the_frame() {
    let scripted = scripted(
        Script {
            version: "0.5.0+ext-v1 worker",
            answer: 0,
            on_remove: vec![Frame::End],
        },
        local_inner(),
    );
    assert!(!vfs::supports_remove_tree(&scripted.backend, "/srv/share/ordner").unwrap());
    let failure = vfs::remove_tree(
        &scripted.backend,
        "/srv/share/ordner",
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(failure.error.kind(), io::ErrorKind::Unsupported);
    assert_eq!((failure.planned, failure.removed), (0, 0));
    let seen = scripted.finish();
    assert!(
        !seen.iter().any(|(_, frame)| is_remove_tree(frame)),
        "{seen:?}"
    );
}

#[test]
fn delete_speed_task_server_without_extensions_is_never_asked() {
    // A server without `ext-v1` cannot even decode the question.
    let scripted = scripted(
        Script {
            version: "0.5.0+sync-links-v1 worker",
            answer: 1,
            on_remove: vec![Frame::End],
        },
        local_inner(),
    );
    assert!(!vfs::supports_remove_tree(&scripted.backend, "/srv").unwrap());
    let seen = scripted.finish();
    assert!(seen
        .iter()
        .all(|(_, frame)| matches!(frame, Frame::Hello { .. })));
}

#[test]
fn delete_speed_task_client_cancel_sends_cancel_and_reports_counters() {
    let scripted = scripted(
        Script {
            version: "0.5.0+ext-v1 worker",
            answer: 1,
            on_remove: vec![Frame::Progress { done: 2, total: 5 }],
        },
        local_inner(),
    );
    let cancel = AtomicBool::new(false);
    let report = vfs::remove_tree(&scripted.backend, "/srv/big", &cancel, &mut |_| {
        cancel.store(true, Ordering::Release);
    })
    .unwrap();
    assert_eq!(
        report,
        RecursiveDeleteReport {
            status: RecursiveDeleteStatus::Canceled,
            planned: 5,
            removed: 2,
        }
    );
    let remove_id = scripted
        .seen
        .lock()
        .unwrap()
        .iter()
        .find(|(_, frame)| is_remove_tree(frame))
        .map(|(id, _)| *id)
        .expect("the deletion request");
    assert!(
        scripted.wait_for(|id, frame| id == remove_id && *frame == Frame::Cancel),
        "the serving side must be told to stop"
    );
    scripted.finish();
}

#[test]
fn delete_speed_task_client_failure_carries_the_last_counters() {
    let scripted = scripted(
        Script {
            version: "0.5.0+ext-v1 worker",
            answer: 1,
            on_remove: vec![
                Frame::Progress { done: 3, total: 7 },
                Frame::Err("permission denied by the test host".into()),
            ],
        },
        local_inner(),
    );
    let failure: RecursiveDeleteFailure = vfs::remove_tree(
        &scripted.backend,
        "/srv/locked",
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!((failure.planned, failure.removed), (7, 3));
    assert_eq!(failure.error.kind(), io::ErrorKind::PermissionDenied);
    assert!(failure.error.to_string().contains("test host"));
    scripted.finish();
}

/// The SFTP connection of an SSH location that deletes folders itself.
struct TreeInner {
    calls: Arc<Mutex<Vec<String>>>,
}

fn unused<T>() -> VfsResult<T> {
    Err(io::Error::other("unused by this test"))
}

impl Backend for TreeInner {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
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

impl BackendExtensions for TreeInner {
    fn supports_remove_tree(&self, _path: &str) -> VfsResult<bool> {
        Ok(true)
    }

    fn remove_tree(
        &self,
        path: &str,
        _cancel: &AtomicBool,
        progress: &mut dyn FnMut(RecursiveDeleteProgress),
    ) -> Result<RecursiveDeleteReport, RecursiveDeleteFailure> {
        self.calls.lock().unwrap().push(path.into());
        progress(RecursiveDeleteProgress {
            phase: RecursiveDeletePhase::Applying,
            planned: 2,
            removed: 2,
            current: path.into(),
        });
        Ok(RecursiveDeleteReport {
            status: RecursiveDeleteStatus::Complete,
            planned: 2,
            removed: 2,
        })
    }
}

#[test]
fn delete_speed_task_old_ssh_agent_falls_back_to_its_sftp_connection() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let inner: BackendHandle = Arc::new(TreeInner {
        calls: calls.clone(),
    });
    let scripted = scripted(
        Script {
            version: "0.5.0+ext-v1",
            answer: 0,
            on_remove: vec![Frame::End],
        },
        inner,
    );
    assert!(vfs::supports_remove_tree(&scripted.backend, "/home/u/old").unwrap());
    let report = vfs::remove_tree(
        &scripted.backend,
        "/home/u/old",
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.status, RecursiveDeleteStatus::Complete);
    assert_eq!(*calls.lock().unwrap(), vec!["/home/u/old".to_string()]);
    let seen = scripted.finish();
    assert!(
        !seen.iter().any(|(_, frame)| is_remove_tree(frame)),
        "{seen:?}"
    );
}

#[test]
fn delete_speed_task_real_agent_deletes_a_folder_in_one_request() {
    let base = std::env::temp_dir().join(format!(
        "se_delete_speed_agent_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let root = base.join("victim");
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    std::fs::write(root.join("a/b/deep.txt"), b"deep").unwrap();
    std::fs::write(root.join("top.txt"), b"top").unwrap();
    std::fs::write(base.join("neighbour.txt"), b"stays").unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        let _ = crate::agent_proto::serve(read, socket);
    });
    let client = TcpStream::connect(address).unwrap();
    let shutdown = client.try_clone().unwrap();
    let read: Box<dyn Read + Send> = Box::new(client.try_clone().unwrap());
    let write: Box<dyn Write + Send> = Box::new(client);
    let backend = AgentBackend::from_streams(read, write, local_inner()).unwrap();

    let path = root.to_string_lossy().into_owned();
    assert!(vfs::supports_remove_tree(&backend, &path).unwrap());
    let mut reports = Vec::new();
    let report = vfs::remove_entry_controlled(
        &backend,
        &folder(&path),
        &AtomicBool::new(false),
        |progress| reports.push(progress),
    )
    .unwrap();
    // Two children plus the folder itself.
    assert_eq!(
        report,
        RecursiveDeleteReport {
            status: RecursiveDeleteStatus::Complete,
            planned: 3,
            removed: 3,
        }
    );
    assert!(!reports.is_empty());
    assert!(!root.exists());
    assert_eq!(std::fs::read(base.join("neighbour.txt")).unwrap(), b"stays");

    // A file is refused and stays.
    let file = base.join("neighbour.txt").to_string_lossy().into_owned();
    let failure =
        vfs::remove_tree(&backend, &file, &AtomicBool::new(false), &mut |_| {}).unwrap_err();
    assert_eq!((failure.planned, failure.removed), (0, 0));
    assert!(base.join("neighbour.txt").exists());

    drop(backend);
    let _ = shutdown.shutdown(Shutdown::Both);
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(base);
}
