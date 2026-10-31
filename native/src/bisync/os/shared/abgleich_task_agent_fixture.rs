//! A real agent server on a local socket, as a sync side sees a remote one.
//! The client's inner backend (its SSH connection in the product) reads the
//! same folders but reports itself as not local: `AgentBackend::is_local`
//! answers from its inner backend (`agent/core/backend.rs`), and a local
//! side is never walked next to its data (`snapshot_agent.rs`, its first
//! guard). The same pattern as `tests/links_remote.rs` (`RemoteClient`).
use crate::agent::AgentBackend;
use crate::vfs::{Backend, BackendHandle, LocalBackend, VfsMeta, VfsResult};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

/// The inner backend of a remote agent: local I/O, not a local side.
struct RemoteInner(LocalBackend);

impl Backend for RemoteInner {
    fn scheme(&self) -> crate::vfs::Scheme {
        self.0.scheme()
    }

    fn root_display(&self) -> String {
        self.0.root_display()
    }

    fn is_local(&self) -> bool {
        false
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.0.list_dir(path)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.0.stat(path)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.0.open_read(path)
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.0.open_write(path)
    }

    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.0.rename(source, destination)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.0.remove_file(path)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.0.remove_dir(path)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.0.mkdir_all(path)
    }
}

/// A client of an agent server serving this device's files over a socket,
/// seen as a remote side.
pub(super) fn remote_agent() -> AgentBackend {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        let _ = crate::agent_proto::serve(read, socket);
    });
    let client = TcpStream::connect(address).unwrap();
    let read: Box<dyn Read + Send> = Box::new(client.try_clone().unwrap());
    let write: Box<dyn Write + Send> = Box::new(client);
    let inner: BackendHandle = Arc::new(RemoteInner(LocalBackend::new("/")));
    AgentBackend::from_streams(read, write, inner).unwrap()
}
