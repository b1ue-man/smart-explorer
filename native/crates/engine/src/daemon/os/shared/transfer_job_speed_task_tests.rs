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

#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture"]
fn speed_task_worker_job_upload_through_the_proxy() {
    let root = tempfile::tempdir().expect("temp dir");
    let tree = tree(root.path());
    let target = root.path().join("target");
    std::fs::create_dir(&target).expect("target");
    let backend = proxied();
    let items = JobItems::Roots {
        paths: vec![fwd(&tree)],
        base: None,
    };
    let folders = FolderConflictPolicy::new(FolderConflict::KeepBoth);
    let target_dir = fwd(&target);
    let id = crate::transfer::new_job_id();
    let view = JobView {
        id: Some(&id),
        recoverable: true,
        recovery: RecoveryMode::New,
        source: Side::Local,
        target: Side::Remote(&*backend),
        target_dir: &target_dir,
        items: &items,
        layout: Layout::Tree,
        filter: None,
        conflict: Conflict::Rename,
        folders: &folders,
        mode: CopyMode::Copy,
        source_label: "Quelle",
        target_label: "Ziel",
        resume: None,
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    crate::transfer::run_view_here(
        view,
        &|message| {
            let _ = tx.send(message);
        },
        &cancel,
    );
    let files_done = rx
        .try_iter()
        .find_map(|message| match message {
            TransferMsg::Done { progress, .. } => Some(progress.files_done),
            TransferMsg::Progress(_) => None,
        })
        .expect("end");
    report("worker_job_proxy", started, files_done);
}

#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture"]
fn speed_task_worker_job_upload_in_the_worker() {
    let root = tempfile::tempdir().expect("temp dir");
    let tree = tree(root.path());
    let target = root.path().join("target");
    std::fs::create_dir(&target).expect("target");
    let opened = OpenedJob {
        wire: JobWire {
            id: Some(crate::transfer::new_job_id()),
            recoverable: true,
            resume_record: false,
            source: EndpointWire::Local,
            // The live endpoint below; the wire form plays no part here.
            target: EndpointWire::Local,
            target_dir: fwd(&target),
            items: JobItems::Roots {
                paths: vec![fwd(&tree)],
                base: None,
            },
            layout: Layout::Tree,
            filter: None,
            conflict: 0,
            folders: FolderConflictPolicy::new(FolderConflict::KeepBoth),
            moving: false,
            source_label: "Quelle".into(),
            target_label: "Ziel".into(),
            resume: None,
            survives_client: true,
        },
        conflict: Conflict::Rename,
        filter: None,
        source: None,
        target: Some(served()),
    };
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let mut client = TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
    let (worker, _) = listener.accept().expect("accept");
    let started = Instant::now();
    let server = std::thread::spawn(move || run(worker, opened));
    let files_done = loop {
        match read_response(&mut client).expect("reply") {
            IpcResponse::Transfer(TransferIpcReply::Done { progress, .. }) => {
                break progress.files_done
            }
            IpcResponse::Transfer(_) => {}
            other => panic!("unexpected response: {other:?}"),
        }
    };
    server.join().expect("worker thread").expect("served");
    report("worker_job_in_worker", started, files_done);
}
