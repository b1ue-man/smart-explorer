//! The app's side of the job service (Z-P2): handles of other places never
//! go to the worker, Share handles are recognised through a browsing cache,
//! a reply line split by a read timeout is kept whole, and a running job's
//! progress, cancel and end travel over IPC.
use super::super::ipc_protocol::transfer::{TransferIpcReply, TransferIpcRequest};
use super::super::ipc_protocol::{write_response, IpcRequest, IpcResponse};
use super::{follow, read_piece, remember_share, run_transfer_in_worker, share_target};
use crate::share::PeerOpenTarget;
use crate::transfer::{
    FolderConflict, FolderConflictPolicy, JobItems, JobView, Layout, RecoveryMode, Side,
    TransferKind, TransferMsg, TransferProgress,
};
use crate::types::{Conflict, CopyMode};
use crate::vfs::BackendHandle;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn target(contact: &str) -> PeerOpenTarget {
    PeerOpenTarget::Direct {
        contact_id: contact.to_string(),
    }
}

fn view<'a>(
    source: Side<'a>,
    target: Side<'a>,
    items: &'a JobItems,
    folders: &'a FolderConflictPolicy,
) -> JobView<'a> {
    JobView {
        id: Some("client-task"),
        recoverable: true,
        recovery: RecoveryMode::New,
        source,
        target,
        target_dir: "/to",
        items,
        layout: Layout::Tree,
        filter: None,
        conflict: Conflict::Rename,
        folders,
        mode: CopyMode::Copy,
        source_label: "Quelle",
        target_label: "Ziel",
        resume: None,
    }
}

#[test]
fn worker_job_task_share_handles_are_known_through_a_browsing_cache() {
    let share: BackendHandle = Arc::new(crate::vfs::LocalBackend::new("/"));
    remember_share(&share, &target("laptop"));
    assert_eq!(share_target(&*share), Some(target("laptop")));
    let cached: BackendHandle = Arc::new(crate::vfs::CachingBackend::new(share.clone()));
    assert_eq!(share_target(&*cached), Some(target("laptop")));
    let other: BackendHandle = Arc::new(crate::vfs::LocalBackend::new("/"));
    assert_eq!(share_target(&*other), None);
    drop(cached);
    drop(share);
    assert_eq!(share_target(&*other), None);
}

#[test]
fn worker_job_task_other_places_never_go_to_the_worker() {
    let items = JobItems::Roots {
        paths: vec!["/from/a".into()],
        base: None,
    };
    let folders = FolderConflictPolicy::new(FolderConflict::KeepBoth);
    let unknown: BackendHandle = Arc::new(crate::vfs::LocalBackend::new("/"));
    let reported = Mutex::new(0usize);
    let report = |_: TransferMsg| {
        *reported.lock().expect("count") += 1;
    };
    let cancel = AtomicBool::new(false);
    for (source, target) in [
        (Side::Local, Side::Local),
        (Side::Local, Side::Remote(&*unknown)),
        (Side::Remote(&*unknown), Side::Local),
    ] {
        let job = view(source, target, &items, &folders);
        assert!(!run_transfer_in_worker(&job, &report, &cancel));
    }
    assert_eq!(*reported.lock().expect("count"), 0, "nothing reported");
}

#[test]
fn worker_job_task_a_line_split_by_a_timeout_stays_whole() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let writer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        stream.write_all(b"{\"first\":").expect("half");
        stream.flush().expect("flush");
        std::thread::sleep(Duration::from_millis(400));
        stream.write_all(b"1}\n{\"second\":2}\n").expect("rest");
    });
    let stream = TcpStream::connect(address).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("timeout");
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    let mut lines = Vec::new();
    let mut timeouts = 0;
    while lines.len() < 2 {
        match read_piece(&mut reader, &mut line) {
            Ok(true) => {
                lines.push(String::from_utf8(std::mem::take(&mut line)).expect("utf8"));
            }
            Ok(false) => panic!("stream ended early"),
            Err(error) if super::waiting(&error) => timeouts += 1,
            Err(error) => panic!("{error}"),
        }
    }
    writer.join().expect("writer");
    assert!(timeouts >= 1, "the reply was split by a timeout");
    assert_eq!(lines, ["{\"first\":1}\n", "{\"second\":2}\n"]);
    assert!(reader
        .fill_buf()
        .map(|rest| rest.is_empty())
        .unwrap_or(true));
}

#[test]
fn worker_job_task_progress_cancel_and_end_travel_over_ipc() {
    // The worker's job stream.
    let jobs = TcpListener::bind("127.0.0.1:0").expect("job listener");
    let client = TcpStream::connect(jobs.local_addr().expect("address")).expect("connect");
    let (mut worker, _) = jobs.accept().expect("accept");
    // The worker's IPC listener, which receives the cancel.
    let control = TcpListener::bind("127.0.0.1:0").expect("control listener");
    let control_address = control.local_addr().expect("address");
    let progress = TransferProgress::new(TransferKind::Upload, "Upload", 3, 30);
    let streamed = progress.clone();
    let canceled_by = std::thread::spawn(move || {
        let send = |worker: &mut TcpStream, reply: TransferIpcReply| {
            write_response(worker, &IpcResponse::Transfer(reply)).expect("reply");
        };
        send(
            &mut worker,
            TransferIpcReply::Progress {
                progress: Box::new(streamed.clone()),
            },
        );
        // The cancel arrives on its own connection.
        let (mut cancel, _) = control.accept().expect("cancel connection");
        let mut line = String::new();
        BufReader::new(cancel.try_clone().expect("clone"))
            .read_line(&mut line)
            .expect("cancel request");
        let request: IpcRequest = serde_json::from_str(line.trim()).expect("request");
        let IpcRequest::Transfer {
            req: TransferIpcRequest::Cancel { id },
            ..
        } = request
        else {
            panic!("not a cancel: {line}");
        };
        send(&mut cancel, TransferIpcReply::Cancelled { found: true });
        let mut done = streamed;
        done.done = true;
        send(
            &mut worker,
            TransferIpcReply::Done {
                progress: Box::new(done),
                errors: Vec::new(),
                canceled: true,
                issues: Vec::new(),
                roots: Vec::new(),
            },
        );
        id
    });
    let items = JobItems::Roots {
        paths: vec!["/from/a".into()],
        base: None,
    };
    let folders = FolderConflictPolicy::new(FolderConflict::KeepBoth);
    let job = view(Side::Local, Side::Local, &items, &folders);
    let cancel = AtomicBool::new(false);
    let messages = Mutex::new(Vec::new());
    let report = |message: TransferMsg| {
        // The user cancels once the first progress is shown.
        if matches!(message, TransferMsg::Progress(_)) {
            cancel.store(true, Ordering::Release);
        }
        messages.lock().expect("messages").push(message);
    };
    follow(
        &job,
        &report,
        &cancel,
        client,
        (control_address, "token", "job-7"),
    );
    assert_eq!(canceled_by.join().expect("worker"), "job-7");
    let messages = messages.into_inner().expect("messages");
    assert_eq!(messages.len(), 2, "one progress, one end");
    match &messages[0] {
        TransferMsg::Progress(shown) => assert_eq!(shown.bytes_total, progress.bytes_total),
        TransferMsg::Done { .. } => panic!("progress first"),
    }
    match &messages[1] {
        TransferMsg::Done {
            canceled, progress, ..
        } => {
            assert!(*canceled);
            assert!(progress.done);
        }
        TransferMsg::Progress(_) => panic!("the end last"),
    }
}
