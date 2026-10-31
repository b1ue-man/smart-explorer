//! The agent's walk next to the data through the real server: `WalkHashed3`
//! with listing attributes (links and special files as entries, folder sizes
//! as listed, hidden dot names), the former item set without them, folders
//! ahead of their content on the host's threads, and a server without
//! `walk-meta-v1` that never gets an attribute request.
use super::AgentBackend;
use crate::agent_proto::{read_frame, write_frame, Frame, PROTO_VERSION};
use crate::vfs::{
    self as vfs, Backend, BackendHandle, HashWalkItem, HashWalkRequest, LocalBackend,
};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

fn connect(serve: impl FnOnce(TcpStream, TcpStream) + Send + 'static) -> AgentBackend {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        serve(read, socket);
    });
    let client = TcpStream::connect(address).unwrap();
    let read: Box<dyn Read + Send> = Box::new(client.try_clone().unwrap());
    let write: Box<dyn Write + Send> = Box::new(client);
    let inner: BackendHandle = Arc::new(LocalBackend::new("/"));
    AgentBackend::from_streams(read, write, inner).unwrap()
}

fn real_agent() -> AgentBackend {
    connect(|read, write| {
        let _ = crate::agent_proto::serve(read, write);
    })
}

fn walk(backend: &AgentBackend, root: &str, request: HashWalkRequest) -> (bool, Vec<HashWalkItem>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    let ran = vfs::hash_walk(backend, root, request, tx, &AtomicBool::new(false)).unwrap();
    (ran, rx.try_iter().collect())
}

/// Forty folders of three files each, a hidden file, a link and a FIFO.
fn tree() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("a.txt"), b"abc").unwrap();
    std::fs::write(root.join(".hidden.txt"), b"h").unwrap();
    for folder in 0..40 {
        let dir = root.join(format!("d{folder}")).join("inner");
        std::fs::create_dir_all(&dir).unwrap();
        for file in 0..3 {
            std::fs::write(dir.join(format!("f{file}.txt")), b"12345").unwrap();
        }
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("a.txt"), root.join("link")).unwrap();
        let fifo = std::ffi::CString::new(root.join("fifo").to_string_lossy().as_bytes()).unwrap();
        // SAFETY: a valid NUL-terminated path; creates one FIFO node.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    }
    directory
}

fn request(meta: bool) -> HashWalkRequest {
    HashWalkRequest {
        algorithm: None,
        min_bytes: 0,
        meta,
    }
}

#[test]
fn abgleich_task_agent_walk_with_meta_lists_links_specials_and_hidden_entries() {
    let directory = tree();
    let root = directory.path().to_string_lossy().into_owned();
    let backend = real_agent();
    let (ran, items) = walk(&backend, &root, request(true));
    assert!(ran);
    let mut position = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        let HashWalkItem::Entry(entry) = item else {
            panic!("no omission expected: {item:?}");
        };
        let meta = entry.meta.expect("every entry carries attributes");
        position.insert(entry.rel.clone(), index);
        if let Some((parent, _)) = entry.rel.rsplit_once('/') {
            assert!(
                position.contains_key(parent),
                "{parent} after {}",
                entry.rel
            );
        }
        if entry.is_dir {
            let listed = std::fs::symlink_metadata(directory.path().join(&entry.rel)).unwrap();
            assert_eq!(entry.size, listed.len(), "folder size as listed");
        }
        assert_eq!(
            meta.hidden,
            entry.rel.rsplit('/').next().unwrap().starts_with('.')
        );
    }
    assert_eq!(position.len(), 2 + 40 * 5 + if cfg!(unix) { 2 } else { 0 });
    #[cfg(unix)]
    {
        let entry = |rel: &str| match &items[position[rel]] {
            HashWalkItem::Entry(entry) => entry.clone(),
            other => panic!("{other:?}"),
        };
        let link = entry("link");
        assert!(!link.is_dir && link.meta.unwrap().is_symlink && !link.is_regular());
        let fifo = entry("fifo");
        assert!(!fifo.is_dir && fifo.meta.unwrap().special && fifo.size == 0);
    }
}

#[test]
fn abgleich_task_agent_walk_without_meta_keeps_the_former_items() {
    let directory = tree();
    let root = directory.path().to_string_lossy().into_owned();
    let backend = real_agent();
    let (ran, items) = walk(
        &backend,
        &root,
        HashWalkRequest {
            algorithm: Some(crate::analytics::HashAlgorithm::Md5),
            ..request(false)
        },
    );
    assert!(ran);
    let mut omitted = Vec::new();
    let mut entries = 0;
    for item in &items {
        match item {
            HashWalkItem::Entry(entry) => {
                entries += 1;
                assert!(entry.meta.is_none());
                if entry.is_dir {
                    assert_eq!(entry.size, 0);
                }
                if entry.rel == "a.txt" {
                    assert_eq!(
                        entry.digest.as_deref(),
                        Some("900150983cd24fb0d6963f7d28e17f72")
                    );
                }
            }
            HashWalkItem::Omitted(omission) => {
                omitted.push((omission.rel.clone(), omission.reason))
            }
        }
    }
    assert_eq!(entries, 2 + 40 * 5);
    if cfg!(unix) {
        omitted.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            omitted,
            [
                ("fifo".to_string(), vfs::OmissionReason::Special),
                ("link".to_string(), vfs::OmissionReason::Link)
            ]
        );
    }
}

#[test]
fn abgleich_task_agent_listing_marks_dot_names_hidden() {
    let directory = tree();
    let root = directory.path().to_string_lossy().into_owned();
    let backend = real_agent();
    let listing = backend.list_dir(&root).unwrap();
    let hidden: HashMap<_, _> = listing
        .iter()
        .map(|m| (m.name.as_str(), m.hidden))
        .collect();
    assert_eq!(hidden[".hidden.txt"], true);
    assert_eq!(hidden["a.txt"], false);
    let tolerant = vfs::list_dir_tolerant(&backend, &root).unwrap();
    assert!(tolerant
        .entries
        .iter()
        .all(|m| m.hidden == m.name.starts_with('.')));
}

#[test]
fn abgleich_task_agent_without_walk_meta_never_gets_an_attribute_request() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    let backend = connect(move |mut read, mut write| {
        while let Ok(Some((id, frame))) = read_frame(&mut read) {
            let reply = match &frame {
                Frame::Hello { .. } => Frame::HelloOk {
                    proto: PROTO_VERSION,
                    version: "0.5.180+sync-links-v1.credit-v1.stage-v1.ext-v1.slots-64".into(),
                },
                _ => Frame::Err("unexpected".into()),
            };
            record.lock().unwrap().push(frame);
            if write_frame(&mut write, id, &reply).is_err() {
                break;
            }
        }
    });
    let (ran, items) = walk(&backend, "/tmp", request(true));
    assert!(!ran);
    assert!(items.is_empty());
    assert!(!seen
        .lock()
        .unwrap()
        .iter()
        .any(|frame| matches!(frame, Frame::WalkHashed2 { .. } | Frame::WalkHashed3 { .. })));
}

#[test]
fn abgleich_task_agent_batch_put_sets_the_named_time() {
    use crate::vfs::{BatchPut, BatchPutOutcome};
    let directory = tempfile::tempdir().unwrap();
    let backend = real_agent();
    assert!(vfs::batch_put_times(&backend));
    let path = |name: &str| directory.path().join(name).to_string_lossy().into_owned();
    let entries = [
        BatchPut {
            path: path("timed.txt"),
            size: 3,
            mtime_ms: Some(1_500_000_000_000),
        },
        BatchPut {
            path: path("plain.txt"),
            size: 2,
            mtime_ms: None,
        },
    ];
    let mut data: &[u8] = b"abcde";
    let outcomes = backend.put_batch(&entries, &mut data).unwrap();
    assert!(outcomes
        .iter()
        .all(|outcome| matches!(outcome, BatchPutOutcome::Published(_))));
    let modified = std::fs::metadata(directory.path().join("timed.txt"))
        .unwrap()
        .modified()
        .unwrap();
    let millis = modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    assert_eq!(millis, 1_500_000_000_000);
    assert_eq!(
        std::fs::read(directory.path().join("plain.txt")).unwrap(),
        b"de"
    );
}

#[cfg(unix)]
#[test]
fn abgleich_task_agent_meta_walk_sends_each_folder_as_one_run() {
    let directory = tempfile::tempdir().unwrap();
    crate::agent::walk_runs_check::run_tree(directory.path());
    let root = directory.path().to_string_lossy().into_owned();
    let backend = real_agent();
    let (ran, items) = walk(&backend, &root, request(true));
    assert!(ran);
    // Root, `big`, 40 folders and their 40 inner folders: one run each.
    let runs = crate::agent::walk_runs_check::assert_folder_runs(&items);
    assert_eq!(runs, 2 + 40 * 2);
    let omitted = items
        .iter()
        .filter(|item| matches!(item, HashWalkItem::Omitted(_)))
        .count();
    assert_eq!(omitted, 5 + 40);
    let entries = items.len() - omitted;
    assert_eq!(entries, 1 + 3000 + 40 * (1 + 1 + 3));
}

/// W5: a stage written with the source's time (`stage-mtime-v1`) carries it
/// when committed, and the commit's report is the stage's evidence (size,
/// time, the identity `FileIdentity` names), so publishing needs neither a
/// `FinishStage` nor a `Stat` of its own.
#[test]
fn abgleich_task_agent_stage_carries_the_source_time_with_its_write() {
    const TIME: i64 = 1_500_000_000_000;
    let directory = tempfile::tempdir().unwrap();
    let backend = real_agent();
    assert!(backend.features().stage_mtime);
    let stage = directory
        .path()
        .join("timed.txt.se-upload-00000000000000aa")
        .to_string_lossy()
        .into_owned();
    let mut writer = vfs::open_write_copy_stage_timed(&backend, &stage, 3, TIME).unwrap();
    writer.write_all(b"abc").unwrap();
    writer.flush().unwrap();
    drop(writer);
    let evidence = vfs::take_stage_evidence(&backend, &stage).expect("the commit's report");
    assert_eq!(evidence.stage.path, stage);
    assert_eq!((evidence.stage.size, evidence.stage.mtime_ms), (3, TIME));
    assert_eq!(
        evidence.stage.object,
        vfs::file_identity(&backend, &stage).unwrap()
    );
    let modified = std::fs::metadata(&stage).unwrap().modified().unwrap();
    let millis = modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    assert_eq!(millis, TIME as u128);
    assert!(
        vfs::take_stage_evidence(&backend, &stage).is_none(),
        "taken once"
    );
}
