//! The background mark of one backend request over the IPC between a drive
//! host (or any other client of the background service) and the daemon
//! (plan 2026-11-14, E4d). The served backend records, for every call,
//! whether the daemon answers it as background work: that is what
//! `PeerBackend::effective_origin` turns into the origin `Job`
//! (`metadata_presence_tests.rs`). Remote suite only (AGENTS.md).
use super::{backend_server::serve_backend, ipc_protocol::MountBackendCapabilities, mount_proxy};
use crate::agent_proto::{read_frame, write_frame, Frame, BACKGROUND_REQUEST, PROTO_VERSION};
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta, VfsResult};
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// The version text of the background service of v0.5.196: no `origin-v1`.
const OLD_SERVICE: &str = "0.5.196+sync-links-v1.credit-v1.stage-v1.ext-v1.walk-meta-v1.walk-filter-v1.slots-64.capacity-v1.folder-reservation-v1.file-identity-v1.list-meta-v1.walk-keys-v1.meta-list-v1 worker";

type Seen = Arc<Mutex<Vec<bool>>>;

/// Answers `try_exists` and notes whether it ran as background work.
struct Recording(Seen);

fn unused<T>() -> VfsResult<T> {
    Err(io::Error::other("unused by this test"))
}

impl Backend for Recording {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn try_exists(&self, _path: &str) -> VfsResult<bool> {
        self.0.lock().unwrap().push(crate::vfs::in_background());
        Ok(true)
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
}

/// One drive: the daemon serves a recording backend on one accepted
/// connection, the host wraps its agent client like a drive host does.
struct Drive {
    proxy: BackendHandle,
    seen: Seen,
    stop: TcpStream,
    daemon: Option<JoinHandle<io::Result<()>>>,
}

impl Drop for Drive {
    fn drop(&mut self) {
        let _ = self.stop.shutdown(Shutdown::Both);
        if let Some(daemon) = self.daemon.take() {
            let _ = daemon.join();
        }
    }
}

fn drive(listener: &TcpListener) -> io::Result<Drive> {
    let seen = Seen::default();
    let served: BackendHandle = Arc::new(Recording(Arc::clone(&seen)));
    let capabilities = MountBackendCapabilities::from_backend(&served);
    let client = TcpStream::connect(listener.local_addr()?)?;
    let stop = client.try_clone()?;
    let (server, _) = listener.accept()?;
    let read = server.try_clone()?;
    let daemon = std::thread::spawn(move || serve_backend(read, server, served));
    let identity: BackendHandle = Arc::new(crate::vfs::LocalBackend::new("/"));
    let agent = crate::agent::AgentBackend::from_streams(
        Box::new(client.try_clone()?),
        Box::new(client),
        identity,
    )?;
    assert!(agent.features().origin, "the service reads the mark");
    Ok(Drive {
        proxy: mount_proxy::wrap(Arc::new(agent), capabilities),
        seen,
        stop,
        daemon: Some(daemon),
    })
}

fn seen(drive: &Drive) -> Vec<bool> {
    drive.seen.lock().unwrap().clone()
}

/// A request made inside a `BackgroundScope` reaches the daemon as
/// background work; the requests before and after it do not.
#[test]
fn verbindungen_task_metadata_presence_the_mark_travels_over_the_ipc_for_one_request(
) -> io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let drive = drive(&listener)?;
    assert!(drive.proxy.try_exists("/a")?, "a program waits");
    {
        let _work = crate::vfs::BackgroundScope::enter();
        assert!(drive.proxy.try_exists("/a")?, "the engine's own work");
    }
    assert!(drive.proxy.try_exists("/a")?, "a program again");
    assert_eq!(seen(&drive), [false, true, false]);
    Ok(())
}

/// Without the mark the handle's origin stays: a client of v0.5.196 sends
/// counted ids without the bit (the daemon answers as before), and towards
/// a service of v0.5.196 (no `origin-v1`) a client sends no mark at all.
#[test]
fn verbindungen_task_metadata_presence_without_the_mark_the_handle_origin_stays() -> io::Result<()>
{
    // An old client against this daemon, frame by frame.
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let seen = Seen::default();
    let served: BackendHandle = Arc::new(Recording(Arc::clone(&seen)));
    let mut client = TcpStream::connect(listener.local_addr()?)?;
    let (server, _) = listener.accept()?;
    let read = server.try_clone()?;
    let daemon = std::thread::spawn(move || serve_backend(read, server, served));
    write_frame(
        &mut client,
        1,
        &Frame::Hello {
            proto: PROTO_VERSION,
        },
    )?;
    match read_frame(&mut client)? {
        Some((1, Frame::HelloOk { version, .. })) => assert!(version.contains(".origin-v1")),
        other => panic!("handshake: {other:?}"),
    }
    for id in [2, 3 | BACKGROUND_REQUEST, 4] {
        write_frame(&mut client, id, &Frame::TryExists("/a".into()))?;
        assert!(matches!(read_frame(&mut client)?, Some((got, Frame::Exists(true))) if got == id));
    }
    assert_eq!(*seen.lock().unwrap(), [false, true, false], "per request");
    client.shutdown(Shutdown::Both)?;
    let _ = daemon.join();

    // This client against a service of v0.5.196.
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let client = TcpStream::connect(listener.local_addr()?)?;
    let (mut server, _) = listener.accept()?;
    let ids = Arc::new(Mutex::new(Vec::new()));
    let asked = Arc::clone(&ids);
    let old = std::thread::spawn(move || -> io::Result<()> {
        while let Some((id, frame)) = read_frame(&mut server)? {
            let reply = match frame {
                Frame::Hello { proto } => Frame::HelloOk {
                    proto,
                    version: OLD_SERVICE.into(),
                },
                Frame::TryExists(_) => {
                    asked.lock().unwrap().push(id);
                    Frame::Exists(true)
                }
                _ => continue,
            };
            write_frame(&mut server, id, &reply)?;
        }
        Ok(())
    });
    let stop = client.try_clone()?;
    let identity: BackendHandle = Arc::new(crate::vfs::LocalBackend::new("/"));
    let agent = crate::agent::AgentBackend::from_streams(
        Box::new(client.try_clone()?),
        Box::new(client),
        identity,
    )?;
    assert!(!agent.features().origin);
    {
        let _work = crate::vfs::BackgroundScope::enter();
        assert!(agent.try_exists("/a")?, "the old service answers as before");
    }
    let asked = ids.lock().unwrap().clone();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        asked[0] & BACKGROUND_REQUEST,
        0,
        "no mark to an old service"
    );
    drop(agent);
    let _ = stop.shutdown(Shutdown::Both);
    let _ = old.join();
    Ok(())
}

/// Each drive's own work travels on that drive's one connection: three
/// drives of one device are three connections, each one `RootedBackend`
/// with one lease at the device (`RootedBackend::new_in`), three of the
/// four a device holds (`share/core/mount_lease.rs`). No drive opens a
/// second connection for its background work.
#[test]
fn verbindungen_task_metadata_presence_a_third_drive_needs_one_lease() -> io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let drives = [drive(&listener)?, drive(&listener)?, drive(&listener)?];
    for drive in &drives {
        assert!(drive.proxy.try_exists("/a")?);
        let _work = crate::vfs::BackgroundScope::enter();
        assert!(drive.proxy.try_exists("/a")?);
    }
    for drive in &drives {
        assert_eq!(seen(drive), [false, true]);
    }
    listener.set_nonblocking(true)?;
    assert_eq!(
        listener.accept().map(|_| ()).map_err(|error| error.kind()),
        Err(io::ErrorKind::WouldBlock),
        "no drive asked for another connection"
    );
    Ok(())
}
