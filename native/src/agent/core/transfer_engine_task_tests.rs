//! Agent protocol over loopback against the real agent server: credit flow
//! control, batches and the stage operations.
// The agent server's batch and stage tests use Unix links and paths.
#![cfg_attr(not(unix), allow(unused_imports))]
use super::AgentBackend;
use crate::agent_proto::BATCH_MAX_FILES;
use crate::vfs::{Backend, BatchGet, BatchPut, BatchPutOutcome, BatchSink, LocalBackend};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(super) fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "se_transfer_engine_{label}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

pub(super) fn fwd(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(super) struct Served {
    pub(super) backend: AgentBackend,
    shutdown: TcpStream,
    server: std::thread::JoinHandle<()>,
}

impl Served {
    pub(super) fn finish(self) {
        drop(self.backend);
        let _ = self.shutdown.shutdown(Shutdown::Both);
        let _ = self.server.join();
    }
}

/// A client over loopback; `serve` runs the server side of one connection.
pub(super) fn served(serve: impl FnOnce(TcpStream, TcpStream) + Send + 'static) -> Served {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        serve(read, socket);
    });
    let client = TcpStream::connect(address).unwrap();
    let shutdown = client.try_clone().unwrap();
    let backend = AgentBackend::from_streams(
        Box::new(client.try_clone().unwrap()),
        Box::new(client),
        Arc::new(LocalBackend::new("/")),
    )
    .unwrap();
    Served {
        backend,
        shutdown,
        server,
    }
}

pub(super) fn serve_agent() -> Served {
    served(|read, socket| {
        let _ = crate::agent_proto::serve(read, socket);
    })
}

/// Open a large read, consume one byte and leave the rest unread; a parallel
/// listing must still answer at once and the stream must stay intact.
pub(super) fn assert_slow_consumer_does_not_block(backend: &AgentBackend, root: &Path) {
    let big = vec![0x5au8; 24 * 1024 * 1024];
    std::fs::write(root.join("big.bin"), &big).unwrap();
    std::fs::write(root.join("small.txt"), b"x").unwrap();
    let mut reader = backend.open_read(&fwd(&root.join("big.bin"))).unwrap();
    let mut first = [0u8; 1];
    reader.read_exact(&mut first).unwrap();
    // Let the server fill this request's window and stop at its credit.
    std::thread::sleep(Duration::from_millis(300));
    let started = Instant::now();
    let listed = backend.list_dir(&fwd(root)).unwrap();
    let waited = started.elapsed();
    assert!(waited < Duration::from_secs(1), "listing waited {waited:?}");
    assert_eq!(listed.len(), 2);
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest).unwrap();
    assert_eq!(rest.len() + 1, big.len());
    assert!(rest.iter().all(|byte| *byte == 0x5a));
    drop(reader);
    assert_eq!(backend.stat(&fwd(&root.join("small.txt"))).unwrap().size, 1);
}

#[derive(Default)]
pub(super) struct RecordingSink {
    pub(super) begun: Vec<(usize, u64)>,
    pub(super) data: HashMap<usize, Vec<u8>>,
    pub(super) ended: Vec<(usize, Option<String>)>,
    pub(super) failed: Vec<usize>,
    pub(super) abort_at: Option<usize>,
}

impl BatchSink for RecordingSink {
    fn begin(&mut self, index: usize, size: u64) -> io::Result<()> {
        if self.abort_at == Some(index) {
            return Err(io::Error::other("sink abort"));
        }
        self.begun.push((index, size));
        Ok(())
    }

    fn data(&mut self, index: usize, bytes: &[u8]) -> io::Result<()> {
        self.data.entry(index).or_default().extend_from_slice(bytes);
        Ok(())
    }

    fn end(&mut self, index: usize, result: io::Result<()>) -> io::Result<()> {
        self.ended
            .push((index, result.err().map(|error| error.to_string())));
        Ok(())
    }

    fn failed(&mut self, index: usize, _error: io::Error) -> io::Result<()> {
        self.failed.push(index);
        Ok(())
    }
}

#[cfg(unix)]
fn stage_left(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().contains(".se-agent-"))
}

#[test]
fn transfer_engine_task_agent_slow_consumer_does_not_block_a_parallel_listing() {
    let root = temp_root("agent_slow");
    let served = serve_agent();
    assert!(served.backend.features().credit);
    assert_slow_consumer_does_not_block(&served.backend, &root);
    served.finish();
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn transfer_engine_task_agent_batches_number_taken_names_and_refuse_links() {
    use std::os::unix::fs::symlink;

    let root = temp_root("agent_batch");
    let dst = root.join("dst");
    std::fs::create_dir_all(&dst).unwrap();
    std::fs::write(dst.join("taken.txt"), b"keep").unwrap();
    let served = serve_agent();
    let limits = served
        .backend
        .batch_limits(&fwd(&dst))
        .expect("agent batches");
    assert_eq!(limits.max_files, BATCH_MAX_FILES);

    let entries = vec![
        BatchPut {
            path: fwd(&dst.join("a.txt")),
            size: 5,
        },
        BatchPut {
            path: fwd(&dst.join("taken.txt")),
            size: 3,
        },
        BatchPut {
            path: fwd(&dst.join("empty")),
            size: 0,
        },
    ];
    let mut data: &[u8] = b"alphanew";
    let outcomes = served.backend.put_batch(&entries, &mut data).unwrap();
    let published: Vec<String> = outcomes
        .into_iter()
        .map(|outcome| match outcome {
            BatchPutOutcome::Published(path) => path,
            BatchPutOutcome::Failed(error) => panic!("entry failed: {error}"),
        })
        .collect();
    assert_eq!(published[0], entries[0].path);
    assert_eq!(published[1], fwd(&dst.join("taken (2).txt")));
    assert_eq!(published[2], entries[2].path);
    assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), b"alpha");
    assert_eq!(std::fs::read(dst.join("taken.txt")).unwrap(), b"keep");
    assert_eq!(std::fs::read(dst.join("taken (2).txt")).unwrap(), b"new");
    assert!(std::fs::read(dst.join("empty")).unwrap().is_empty());
    assert!(!stage_left(&dst));

    // A destination below a link is refused; nothing lands behind it.
    symlink(&dst, root.join("linkdir")).unwrap();
    let evil = [BatchPut {
        path: fwd(&root.join("linkdir").join("evil.txt")),
        size: 1,
    }];
    let mut data: &[u8] = b"e";
    let outcomes = served.backend.put_batch(&evil, &mut data).unwrap();
    assert!(matches!(&outcomes[..], [BatchPutOutcome::Failed(_)]));
    assert!(!dst.join("evil.txt").exists());
    assert!(!stage_left(&dst));

    symlink(dst.join("a.txt"), dst.join("link")).unwrap();
    let items: Vec<BatchGet> = ["a.txt", "link", "missing", "taken (2).txt"]
        .iter()
        .map(|name| BatchGet {
            path: fwd(&dst.join(name)),
            id: None,
            size: 5,
        })
        .collect();
    let mut sink = RecordingSink::default();
    served.backend.get_batch(&items, &mut sink).unwrap();
    assert_eq!(sink.begun, vec![(0, 5), (3, 3)]);
    assert_eq!(sink.data[&0], b"alpha");
    assert_eq!(sink.data[&3], b"new");
    assert_eq!(sink.ended, vec![(0, None), (3, None)]);
    assert_eq!(sink.failed, vec![1, 2]);
    served.finish();
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
struct FailingReader {
    data: Vec<u8>,
    pos: usize,
    fail_at: usize,
}

#[cfg(unix)]
impl Read for FailingReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.fail_at {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Quelle geändert",
            ));
        }
        let count = out.len().min(self.fail_at - self.pos);
        out[..count].copy_from_slice(&self.data[self.pos..self.pos + count]);
        self.pos += count;
        Ok(count)
    }
}

#[cfg(unix)]
#[test]
fn transfer_engine_task_agent_batch_source_failure_discards_only_that_entry() {
    let root = temp_root("agent_batch_fail");
    let served = serve_agent();
    let entries: Vec<BatchPut> = [("a", 4), ("b", 6), ("c", 2)]
        .iter()
        .map(|(name, size)| BatchPut {
            path: fwd(&root.join(name)),
            size: *size,
        })
        .collect();
    let mut data = FailingReader {
        data: b"aaaabbbbbbcc".to_vec(),
        pos: 0,
        fail_at: 7,
    };
    let outcomes = served.backend.put_batch(&entries, &mut data).unwrap();
    assert!(matches!(&outcomes[0], BatchPutOutcome::Published(path) if *path == entries[0].path));
    assert!(matches!(
        &outcomes[1],
        BatchPutOutcome::Failed(error) if error.kind() == io::ErrorKind::InvalidData
    ));
    assert!(matches!(
        &outcomes[2],
        BatchPutOutcome::Failed(error) if error.kind() == io::ErrorKind::UnexpectedEof
    ));
    assert_eq!(std::fs::read(root.join("a")).unwrap(), b"aaaa");
    assert!(!root.join("b").exists());
    assert!(!root.join("c").exists());
    assert!(!stage_left(&root));

    // A sink that aborts cancels the download; the connection stays usable.
    let items = vec![
        BatchGet {
            path: fwd(&root.join("a")),
            id: None,
            size: 4,
        },
        BatchGet {
            path: fwd(&root.join("a")),
            id: None,
            size: 4,
        },
    ];
    let mut sink = RecordingSink {
        abort_at: Some(1),
        ..RecordingSink::default()
    };
    let error = served.backend.get_batch(&items, &mut sink).unwrap_err();
    assert_eq!(error.to_string(), "sink abort");
    assert_eq!(sink.data[&0], b"aaaa");
    assert_eq!(served.backend.list_dir(&fwd(&root)).unwrap().len(), 1);
    served.finish();
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn transfer_engine_task_agent_stage_operations_stay_exclusive() {
    use std::os::unix::fs::symlink;

    let root = temp_root("agent_stage");
    std::fs::write(root.join("a.txt"), b"alpha").unwrap();
    let served = serve_agent();
    let backend = &served.backend;
    let sub = fwd(&root.join("sub"));
    backend.create_dir(&sub).unwrap();
    backend.create_dir(&sub).unwrap();
    assert_eq!(
        backend.create_dir_new(&sub).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    symlink(root.join("sub"), root.join("dirlink")).unwrap();
    assert!(backend.create_dir(&fwd(&root.join("dirlink"))).is_err());

    let stage = fwd(&root.join("copy.txt.se-copy-1"));
    let source = fwd(&root.join("a.txt"));
    assert_eq!(
        backend
            .server_copy_to_stage(
                &source,
                &stage,
                5,
                &std::sync::atomic::AtomicBool::new(false)
            )
            .unwrap(),
        Some(5)
    );
    backend
        .promote_copy_stage(&stage, &fwd(&root.join("copy.txt")))
        .unwrap();
    assert_eq!(std::fs::read(root.join("copy.txt")).unwrap(), b"alpha");
    let wrong = fwd(&root.join("wrong.se-copy-2"));
    assert!(backend
        .server_copy_to_stage(
            &source,
            &wrong,
            4,
            &std::sync::atomic::AtomicBool::new(false)
        )
        .is_err());

    // Only stage names may be discarded; a published file stays.
    assert!(backend
        .discard_copy_stage(&fwd(&root.join("copy.txt")))
        .is_err());
    assert!(root.join("copy.txt").exists());
    let written = fwd(&root.join("fresh.bin.se-copy-3"));
    let mut writer = backend.open_write_copy_stage(&written).unwrap();
    writer.write_all(b"partial").unwrap();
    writer.flush().unwrap();
    backend.discard_copy_stage(&written).unwrap();
    assert!(!root.join("fresh.bin.se-copy-3").exists());

    let mut tail = String::new();
    backend
        .open_read_at(&source, None, 2)
        .unwrap()
        .expect("agents read from an offset")
        .read_to_string(&mut tail)
        .unwrap();
    assert_eq!(tail, "pha");
    assert!(backend.flow_key(&source).starts_with("agent:"));
    assert_eq!(backend.transfer_ceiling(&source), Some(62));
    served.finish();
    let _ = std::fs::remove_dir_all(root);
}

/// The agent binary carries its own copy of the numbering; it must name
/// taken files exactly like every other transfer (K18c).
#[test]
fn transfer_engine_task_numbered_names_match_the_vfs_helper() {
    for name in ["a.txt", "archive.tar.gz", ".hidden", "plain", "dot.", ""] {
        for index in [1, 2, 17] {
            assert_eq!(
                crate::agent_proto::numbered_name(name, index),
                crate::vfs::remote_util::numbered_remote_name(name, index)
            );
        }
    }
}
