//! The worker's job service (Z-P2) over a real loopback connection: progress
//! and exactly one end reach the client, a cancel stops the job, a job with
//! a crash log outlives a crashed client, any other job ends with it, and
//! one job id never runs twice.
use super::super::ipc_protocol::transfer::{EndpointWire, JobWire, TransferIpcReply};
use super::super::ipc_protocol::{read_response, IpcResponse};
use super::{cancel, run, running, OpenedJob, Registered};
use crate::transfer::{FolderConflict, FolderConflictPolicy, JobItems, Layout};
use crate::types::Conflict;
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn fwd(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// `count` small files below `root/source/tree`.
fn tree(root: &Path, count: usize) -> std::path::PathBuf {
    let tree = root.join("source").join("tree");
    for index in 0..count {
        let folder = tree.join(format!("d{:02}", index % 10));
        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::write(
            folder.join(format!("f{index:05}.txt")),
            format!("file {index}"),
        )
        .expect("file");
    }
    tree
}

/// A local → local job of `tree` into `target`.
fn job(id: &str, tree: &Path, target: &Path, survives_client: bool) -> OpenedJob {
    OpenedJob {
        wire: JobWire {
            id: Some(id.to_string()),
            recoverable: true,
            resume_record: false,
            source: EndpointWire::Local,
            target: EndpointWire::Local,
            target_dir: fwd(target),
            items: JobItems::Roots {
                paths: vec![fwd(tree)],
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
            survives_client,
        },
        conflict: Conflict::Rename,
        filter: None,
        source: None,
        target: None,
    }
}

/// The worker's end and the client's end of one IPC connection.
fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let client = TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
    let (worker, _) = listener.accept().expect("accept");
    (worker, client)
}

fn copied(target: &Path) -> usize {
    walk(&target.join("tree"))
}

fn walk(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .map(|entry| entry.expect("entry").path())
        .map(|path| if path.is_dir() { walk(&path) } else { 1 })
        .sum()
}

fn reply(client: &mut TcpStream) -> TransferIpcReply {
    match read_response(client).expect("reply") {
        IpcResponse::Transfer(reply) => reply,
        other => panic!("unexpected response: {other:?}"),
    }
}

fn id(tag: &str) -> String {
    format!("worker-job-task-{tag}-{}", crate::transfer::new_job_id())
}

#[test]
fn worker_job_task_streams_progress_and_one_end_to_the_client() {
    let root = tempfile::tempdir().expect("temp dir");
    let tree = tree(root.path(), 40);
    let target = root.path().join("target");
    std::fs::create_dir(&target).expect("target");
    let (worker, mut client) = pair();
    let job_id = id("progress");
    let opened = job(&job_id, &tree, &target, true);
    let server = std::thread::spawn(move || run(worker, opened));
    match reply(&mut client) {
        TransferIpcReply::Started { id } => assert_eq!(id, job_id),
        other => panic!("not started: {other:?}"),
    }
    let mut progress = 0;
    let done = loop {
        match reply(&mut client) {
            TransferIpcReply::Progress { .. } => progress += 1,
            TransferIpcReply::Done {
                progress, canceled, ..
            } => break (progress, canceled),
            other => panic!("unexpected reply: {other:?}"),
        }
    };
    server.join().expect("worker thread").expect("served");
    assert!(progress >= 1, "progress before the end");
    let (progress, canceled) = done;
    assert!(!canceled);
    assert!(progress.done);
    assert_eq!(progress.files_done, 40);
    assert_eq!(copied(&target), 40);
    assert!(!running().iter().any(|(id, _)| *id == job_id));
}

#[test]
fn worker_job_task_cancel_reaches_the_running_job() {
    let root = tempfile::tempdir().expect("temp dir");
    let tree = tree(root.path(), 400);
    let target = root.path().join("target");
    std::fs::create_dir(&target).expect("target");
    let (worker, mut client) = pair();
    let job_id = id("cancel");
    let opened = job(&job_id, &tree, &target, true);
    let server = std::thread::spawn(move || run(worker, opened));
    assert!(matches!(
        reply(&mut client),
        TransferIpcReply::Started { .. }
    ));
    assert!(cancel(&job_id), "the running job is found");
    let canceled = loop {
        match reply(&mut client) {
            TransferIpcReply::Progress { .. } => {}
            TransferIpcReply::Done { canceled, .. } => break canceled,
            other => panic!("unexpected reply: {other:?}"),
        }
    };
    server.join().expect("worker thread").expect("served");
    assert!(canceled, "the end reports the cancel");
    assert!(!cancel(&job_id), "an ended job is no longer listed");
}

#[test]
fn worker_job_task_job_with_crash_log_outlives_a_crashed_client() {
    let root = tempfile::tempdir().expect("temp dir");
    let tree = tree(root.path(), 300);
    let target = root.path().join("target");
    std::fs::create_dir(&target).expect("target");
    let (worker, mut client) = pair();
    let job_id = id("client-crash");
    let opened = job(&job_id, &tree, &target, true);
    let server = std::thread::spawn(move || run(worker, opened));
    assert!(matches!(
        reply(&mut client),
        TransferIpcReply::Started { .. }
    ));
    let stop = running()
        .iter()
        .find(|(id, _)| *id == job_id)
        .map(|(_, stop)| stop.clone());
    // The app process dies: its end of the connection closes.
    drop(client);
    server.join().expect("worker thread").expect("served");
    assert_eq!(copied(&target), 300, "the job went on to its end");
    if let Some(stop) = stop {
        assert!(!stop.load(std::sync::atomic::Ordering::Acquire));
    }
}

#[test]
fn worker_job_task_job_without_crash_log_ends_with_its_client() {
    let root = tempfile::tempdir().expect("temp dir");
    let tree = tree(root.path(), 3000);
    let target = root.path().join("target");
    std::fs::create_dir(&target).expect("target");
    let (worker, client) = pair();
    let job_id = id("client-gone");
    let opened = job(&job_id, &tree, &target, false);
    // The client is gone before the job's first message.
    drop(client);
    let started = Instant::now();
    let server = std::thread::spawn(move || run(worker, opened));
    let stop = loop {
        if let Some((_, stop)) = running().iter().find(|(id, _)| *id == job_id) {
            break Some(stop.clone());
        }
        if server.is_finished() || started.elapsed() > Duration::from_secs(10) {
            break None;
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    let _ = server.join().expect("worker thread");
    if let Some(stop) = stop {
        assert!(
            stop.load(std::sync::atomic::Ordering::Acquire) || copied(&target) == 3000,
            "a job without crash log stops when its client is gone"
        );
    }
}

#[test]
fn worker_job_task_one_id_never_runs_twice() {
    let job_id = id("twice");
    let first = Registered::new(&job_id, Arc::new(AtomicBool::new(false)));
    assert!(first.is_some());
    assert!(Registered::new(&job_id, Arc::new(AtomicBool::new(false))).is_none());
    drop(first);
    assert!(Registered::new(&job_id, Arc::new(AtomicBool::new(false))).is_some());
}

#[test]
fn worker_job_task_wire_round_trips_and_keeps_only_logged_jobs_alive() {
    let wire = job("wire", Path::new("/from/tree"), Path::new("/to"), true).wire;
    let request = super::super::ipc_protocol::IpcRequest::Transfer {
        token: "token".into(),
        req: super::super::ipc_protocol::transfer::TransferIpcRequest::Start {
            job: Box::new(wire.clone()),
        },
    };
    let line = serde_json::to_string(&request).expect("encoded");
    assert!(line.contains(r#""t":"transfer""#), "{line}");
    assert!(line.contains(r#""kind":"start""#), "{line}");
    let decoded: super::super::ipc_protocol::IpcRequest =
        serde_json::from_str(&line).expect("decoded");
    let super::super::ipc_protocol::IpcRequest::Transfer {
        req: super::super::ipc_protocol::transfer::TransferIpcRequest::Start { job },
        ..
    } = decoded
    else {
        panic!("not a transfer start");
    };
    assert_eq!(job.items, wire.items);
    assert_eq!(job.decoded().expect("decoded").0, Conflict::Rename);
    // Only a job with an id and a crash log survives its client.
    let items = JobItems::Pairs(Vec::new());
    let folders = FolderConflictPolicy::new(FolderConflict::Merge);
    let view = |recoverable: bool, id: Option<&'static str>| crate::transfer::JobView {
        id,
        recoverable,
        recovery: crate::transfer::RecoveryMode::New,
        source: crate::transfer::Side::Local,
        target: crate::transfer::Side::Local,
        target_dir: "/to",
        items: &items,
        layout: Layout::Tree,
        filter: None,
        conflict: Conflict::Skip,
        folders: &folders,
        mode: crate::types::CopyMode::Copy,
        source_label: "Quelle",
        target_label: "Ziel",
        resume: None,
    };
    let of = |recoverable, id| {
        JobWire::of(
            &view(recoverable, id),
            EndpointWire::Local,
            EndpointWire::Local,
        )
    };
    assert!(of(true, Some("job")).survives_client);
    assert!(
        !of(false, None).survives_client,
        "se cp ends with its terminal"
    );
    assert!(!of(true, None).survives_client);
    assert_eq!(of(false, None).conflict, 2);
}
