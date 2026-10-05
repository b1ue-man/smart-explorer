//! Pure peer identity fallbacks over the real AgentBackend extension dispatch.
//! Only the normal Hello handshake is accepted; no filesystem RPC is served.
use crate::agent_proto::{read_frame, write_frame, Frame, PROTO_VERSION};
use crate::share::PeerOpenTarget;
use crate::vfs::{self, Backend, BackendHandle, CachingBackend};
use std::io::{self, Cursor, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::Duration;

#[derive(Default)]
struct WireFacts {
    hello: AtomicUsize,
    filesystem: AtomicUsize,
    closed: Mutex<u8>,
    changed: Condvar,
}

impl WireFacts {
    fn closed(&self) {
        *self.closed.lock().unwrap() += 1;
        self.changed.notify_all();
    }
}

struct ReplyReader {
    incoming: mpsc::Receiver<Option<Vec<u8>>>,
    current: Cursor<Vec<u8>>,
    facts: Arc<WireFacts>,
}

impl Read for ReplyReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            let count = self.current.read(bytes)?;
            if count != 0 {
                return Ok(count);
            }
            match self.incoming.recv() {
                Ok(Some(reply)) => self.current = Cursor::new(reply),
                Ok(None) | Err(_) => return Ok(0),
            }
        }
    }
}

impl Drop for ReplyReader {
    fn drop(&mut self) {
        self.facts.closed();
    }
}

struct HelloWriter {
    incoming: mpsc::Sender<Option<Vec<u8>>>,
    pending: Vec<u8>,
    facts: Arc<WireFacts>,
}

impl Write for HelloWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        while self.pending.len() >= 4 {
            let size = u32::from_le_bytes(self.pending[..4].try_into().unwrap()) as usize;
            if size > 4096 {
                return Err(io::Error::other(
                    "offline Hello fixture received oversized frame",
                ));
            }
            if self.pending.len() < size + 4 {
                break;
            }
            let (id, frame) = read_frame(&mut Cursor::new(&self.pending[..size + 4]))?
                .expect("complete existing agent frame");
            drop(self.pending.drain(..size + 4));
            let response = match frame {
                Frame::Hello { proto } if proto == PROTO_VERSION => {
                    self.facts.hello.fetch_add(1, Ordering::SeqCst);
                    Frame::HelloOk {
                        proto,
                        // A bare version advertises no service or feature labels.
                        version: env!("CARGO_PKG_VERSION").split('+').next().unwrap().into(),
                    }
                }
                _ => {
                    self.facts.filesystem.fetch_add(1, Ordering::SeqCst);
                    Frame::Err("offline identity fixture forbids filesystem RPC".into())
                }
            };
            let mut reply = Vec::new();
            write_frame(&mut reply, id, &response)?;
            self.incoming
                .send(Some(reply))
                .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for HelloWriter {
    fn drop(&mut self) {
        self.facts.closed();
    }
}

struct OfflineAgent {
    backend: Option<BackendHandle>,
    incoming: mpsc::Sender<Option<Vec<u8>>>,
    facts: Arc<WireFacts>,
}

impl OfflineAgent {
    fn new(target: PeerOpenTarget) -> Self {
        let facts = Arc::new(WireFacts::default());
        let (incoming, rx) = mpsc::channel();
        let mut fixture = Self {
            backend: None,
            incoming,
            facts,
        };
        let reader = ReplyReader {
            incoming: rx,
            current: Cursor::new(Vec::new()),
            facts: fixture.facts.clone(),
        };
        let writer = HelloWriter {
            incoming: fixture.incoming.clone(),
            pending: Vec::new(),
            facts: fixture.facts.clone(),
        };
        let inner =
            super::super::ipc_client::share_backend_identity("C08 offline peer".into(), target);
        let agent =
            crate::agent::AgentBackend::from_streams(Box::new(reader), Box::new(writer), inner)
                .expect("normal framed AgentBackend Hello handshake");
        fixture.backend = Some(Arc::new(agent));
        fixture
    }

    fn handle(&self) -> BackendHandle {
        self.backend.as_ref().unwrap().clone()
    }

    fn assert_metadata_only(&self) {
        assert!(self.facts.hello.load(Ordering::SeqCst) >= 1);
        assert_eq!(self.facts.filesystem.load(Ordering::SeqCst), 0);
    }
}

impl Drop for OfflineAgent {
    fn drop(&mut self) {
        drop(self.backend.take());
        let _ = self.incoming.send(None);
        let (closed, _) = self
            .facts
            .changed
            .wait_timeout_while(
                self.facts.closed.lock().unwrap(),
                Duration::from_secs(10),
                |closed| *closed != 2,
            )
            .unwrap();
        let complete = *closed == 2;
        drop(closed);
        if !complete {
            if std::thread::panicking() {
                eprintln!("C08 offline agent reader/writer did not close");
            } else {
                panic!("C08 offline agent reader/writer did not close");
            }
        }
    }
}

fn targets() -> [PeerOpenTarget; 2] {
    [
        PeerOpenTarget::Direct {
            contact_id: "c08-offline-direct".into(),
        },
        PeerOpenTarget::RoomDevice {
            room_id: "c08-offline-room".into(),
            device_id: "c08-offline-device".into(),
        },
    ]
}

#[test]
fn sync_reliability_task_old_jobs_ipc_literal_child_is_finite_without_worker_rpc() {
    for target in targets() {
        let fixture = OfflineAgent::new(target);
        let agent = fixture.handle();
        let cached = CachingBackend::new(agent.clone());
        for backend in [&*agent, &cached as &dyn Backend] {
            let identity = backend.state_identity();
            for _ in 0..32 {
                assert_eq!(
                    vfs::sync_child_path(backend, "/literal%20", "space name [id=literal]")
                        .unwrap(),
                    "/literal%20/space name [id=literal]"
                );
            }
            for invalid in ["", ".", "..", "sub/name", "nul\0name"] {
                assert_eq!(
                    vfs::sync_child_path(backend, "/", invalid)
                        .unwrap_err()
                        .kind(),
                    io::ErrorKind::InvalidInput
                );
            }
            assert_eq!(backend.state_identity(), identity);
        }
        fixture.assert_metadata_only();
    }
}

#[test]
fn sync_reliability_task_old_jobs_ipc_previous_identity_is_finite_and_unproven() {
    for target in targets() {
        let fixture = OfflineAgent::new(target);
        let agent = fixture.handle();
        let cached = CachingBackend::new(agent.clone());
        for backend in [&*agent, &cached as &dyn Backend] {
            let identity = backend.state_identity();
            for _ in 0..32 {
                assert!(vfs::previous_state_identities(backend).unwrap().is_empty());
            }
            assert_eq!(backend.state_identity(), identity);
        }
        fixture.assert_metadata_only();
    }
}

#[test]
fn sync_reliability_task_old_jobs_ipc_reversible_fallback_is_finite_without_mutation() {
    for target in targets() {
        let fixture = OfflineAgent::new(target);
        let agent = fixture.handle();
        let cached = CachingBackend::new(agent.clone());
        for backend in [&*agent, &cached as &dyn Backend] {
            for _ in 0..32 {
                assert!(!vfs::replace_staged_reversible(
                    backend,
                    "/Docs/file.se-transfer-0123456789abcdef",
                    "/Docs/file",
                    "/Docs/.se-replace-fedcba9876543210",
                )
                .unwrap());
            }
            assert_eq!(
                vfs::replace_staged_reversible(
                    backend,
                    "/Docs/file.se-transfer-0123456789abcdef",
                    "/Docs/file",
                    "/Docs/retained",
                )
                .unwrap_err()
                .kind(),
                io::ErrorKind::InvalidInput
            );
        }
        fixture.assert_metadata_only();
    }
}
