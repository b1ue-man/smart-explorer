//! The background service's backend server over loopback: the slow consumer and
//! the batches split by peer limits. Moved here from
//! agent/core/transfer_engine_task_service_tests.rs: the daemon's sync-link
//! fixture lies above the transfer layer (plan docs/plaene/2026-11-01-idealzustand, D13).
use crate::agent::transfer_engine_task_tests::{
    assert_slow_consumer_does_not_block, fwd, served, temp_root, RecordingSink, Served,
};
use crate::vfs::{
    Backend, BackendHandle, BatchGet, BatchLimits, BatchPut, BatchPutOutcome, BatchSink,
    LocalBackend, Scheme, VfsMeta, VfsResult,
};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn serve_service(backend: BackendHandle) -> Served {
    served(move |read, socket| {
        let _ = crate::daemon::serve_sync_link_fixture(read, socket, backend);
    })
}

/// A peer with batches: two files per batch, written through the local
/// filesystem.
struct BatchingLocal {
    inner: LocalBackend,
    puts: AtomicUsize,
}

impl Backend for BatchingLocal {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.inner.list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.inner.stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.inner.rename(src, dst)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }
    fn batch_limits(&self, _dir: &str) -> Option<BatchLimits> {
        Some(BatchLimits {
            max_files: 2,
            max_bytes: 1 << 20,
        })
    }
    fn put_batch(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
    ) -> VfsResult<Vec<BatchPutOutcome>> {
        self.puts.fetch_add(1, Ordering::SeqCst);
        let mut outcomes = Vec::new();
        for entry in entries {
            let mut bytes = vec![0u8; entry.size as usize];
            data.read_exact(&mut bytes)?;
            let mut writer = self.inner.open_write_new(&entry.path)?;
            writer.write_all(&bytes)?;
            writer.flush()?;
            outcomes.push(BatchPutOutcome::Published(entry.path.clone()));
        }
        Ok(outcomes)
    }
    fn get_batch(&self, items: &[BatchGet], sink: &mut dyn BatchSink) -> VfsResult<()> {
        for (index, item) in items.iter().enumerate() {
            let mut bytes = Vec::new();
            let read = self
                .inner
                .open_read(&item.path)
                .and_then(|mut reader| reader.read_to_end(&mut bytes));
            match read {
                Ok(_) => {
                    sink.begin(index, bytes.len() as u64)?;
                    sink.data(index, &bytes)?;
                    sink.end(index, Ok(()))?;
                }
                Err(error) => sink.failed(index, error)?,
            }
        }
        Ok(())
    }
}

#[test]
fn transfer_engine_task_service_slow_consumer_does_not_block_a_parallel_listing() {
    let root = temp_root("service_slow");
    let served = serve_service(Arc::new(LocalBackend::new("/")));
    let features = served.backend.features();
    assert!(features.credit && features.service);
    // The local peer has no batches: the service does not announce them.
    assert!(served.backend.batch_limits(&fwd(&root)).is_none());
    // The service's slots minus its browsing reserve of four.
    let peer_ceiling = LocalBackend::new("/").transfer_ceiling("/");
    let expected = crate::agent_proto::service_slots(peer_ceiling) - 4;
    assert_eq!(served.backend.transfer_ceiling("/"), Some(expected));
    assert_slow_consumer_does_not_block(&served.backend, &root);
    served.finish();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn transfer_engine_task_service_forwards_batches_split_by_peer_limits() {
    let root = temp_root("service_batch");
    let peer = Arc::new(BatchingLocal {
        inner: LocalBackend::new("/"),
        puts: AtomicUsize::new(0),
    });
    let served = serve_service(peer.clone());
    assert!(served.backend.batch_limits(&fwd(&root)).is_some());
    let entries: Vec<BatchPut> = [("x", 1), ("y", 2), ("z", 3)]
        .iter()
        .map(|(name, size)| BatchPut {
            path: fwd(&root.join(name)),
            size: *size,
            mtime_ms: None,
        })
        .collect();
    let mut data: &[u8] = b"xyyzzz";
    let outcomes = served.backend.put_batch(&entries, &mut data).unwrap();
    for (outcome, entry) in outcomes.iter().zip(&entries) {
        assert!(matches!(outcome, BatchPutOutcome::Published(path) if *path == entry.path));
    }
    assert_eq!(peer.puts.load(Ordering::SeqCst), 2);
    assert_eq!(std::fs::read(root.join("z")).unwrap(), b"zzz");

    let items: Vec<BatchGet> = ["x", "y", "z", "missing"]
        .iter()
        .map(|name| BatchGet {
            path: fwd(&root.join(name)),
            id: None,
            size: 3,
        })
        .collect();
    let mut sink = RecordingSink::default();
    served.backend.get_batch(&items, &mut sink).unwrap();
    assert_eq!(sink.begun, vec![(0, 1), (1, 2), (2, 3)]);
    assert_eq!(sink.data[&1], b"yy");
    assert_eq!(sink.failed, vec![3]);
    served.finish();
    let _ = std::fs::remove_dir_all(root);
}
