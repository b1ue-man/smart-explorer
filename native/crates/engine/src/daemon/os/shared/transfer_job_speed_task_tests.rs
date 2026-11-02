//! Benchmarks of Z-P2 (2026-10-28): one upload of 2000 files of 4 KiB to a
//! place the worker serves, once as before (the engine in the app, every
//! request through the worker's loopback proxy) and once in the worker's
//! job service (the engine next to the backend, only progress over IPC).
//! Ignored by default: `cargo test --release -- --ignored
//! speed_task_worker_job --nocapture --test-threads=1` prints one
//! `BENCH <case> files_per_s=<n>` line each.
use super::super::backend_server::serve_backend;
use super::super::ipc_protocol::transfer::{EndpointWire, JobWire, TransferIpcReply};
use super::super::ipc_protocol::{read_response, IpcResponse};
use super::{run, OpenedJob};
use crate::transfer::{
    FolderConflict, FolderConflictPolicy, JobItems, JobView, Layout, RecoveryMode, Side,
    TransferMsg,
};
use crate::types::{Conflict, CopyMode};
use crate::vfs::BackendHandle;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

const FILES: usize = 2000;

fn fwd(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn tree(root: &Path) -> PathBuf {
    let tree = root.join("source").join("tree");
    let bytes = vec![0x3Cu8; 4096];
    for index in 0..FILES {
        let folder = tree.join(format!("d{:02}", index % 20));
        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::write(folder.join(format!("f{index:05}.bin")), &bytes).expect("file");
    }
    tree
}

fn served() -> BackendHandle {
    Arc::new(crate::vfs::LocalBackend::new("/"))
}

/// The place behind the worker's proxy, as the app reaches it.
fn proxied() -> BackendHandle {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    std::thread::spawn(move || {
        let (socket, _) = listener.accept().expect("accept");
        let read = socket.try_clone().expect("clone");
        let _ = serve_backend(read, socket, served());
    });
    let client = TcpStream::connect(address).expect("connect");
    let read: Box<dyn Read + Send> = Box::new(client.try_clone().expect("clone"));
    let write: Box<dyn Write + Send> = Box::new(client);
    let agent = crate::agent::AgentBackend::from_streams(read, write, served()).expect("agent");
    Arc::new(agent)
}

fn report(case: &str, started: Instant, files_done: u64) {
    assert_eq!(files_done, FILES as u64);
    let seconds = started.elapsed().as_secs_f64().max(1e-9);
    println!("BENCH {case} files_per_s={:.0}", FILES as f64 / seconds);
}
