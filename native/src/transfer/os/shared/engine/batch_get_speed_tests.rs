//! Download packets against a fake peer that refuses them.
use super::super::batch_put::speed_tests::forward;
use super::super::test_backend::{job, run, write, Fake};
use crate::transfer::Endpoint;
use crate::vfs::{
    Backend, BackendHandle, BatchGet, BatchLimits, BatchSink, Scheme, VfsMeta, VfsResult,
};
use std::io::{self, Read, Write};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// A peer that offers packets but refuses every `get_batch`.
struct NoPackets(Arc<Fake>);

impl Backend for NoPackets {
    forward! {
        scheme() -> Scheme; root_display() -> String; namespace_identity() -> String;
        list_dir(path: &str) -> VfsResult<Vec<VfsMeta>>; stat(path: &str) -> VfsResult<VfsMeta>;
        open_read(path: &str) -> VfsResult<Box<dyn Read + Send>>;
        open_write(path: &str) -> VfsResult<Box<dyn Write + Send>>;
        rename(src: &str, dst: &str) -> VfsResult<()>;
        remove_file(path: &str) -> VfsResult<()>; remove_dir(path: &str) -> VfsResult<()>;
        mkdir_all(path: &str) -> VfsResult<()>; flow_key(path: &str) -> String;
        batch_limits(dir: &str) -> Option<BatchLimits>;
    }

    fn get_batch(&self, _items: &[BatchGet], _sink: &mut dyn BatchSink) -> VfsResult<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "fixture: no packets",
        ))
    }
}

#[test]
fn speed_task_batch_get_unsupported_falls_back_to_single_files() {
    let root = tempfile::tempdir().expect("temp dir");
    let remote = root.path().join("remote/Vault");
    for index in 0..6 {
        write(
            &remote.join(format!("f{index}.txt")),
            format!("content {index}").as_bytes(),
        );
    }
    let dest = root.path().join("dest");
    std::fs::create_dir(&dest).expect("dest");
    let mut fake = Fake::new("get-unsupported");
    fake.batch = Some(BatchLimits {
        max_files: 64,
        max_bytes: 1024 * 1024,
    });
    let (fake, _) = fake.handle();
    let handle: BackendHandle = Arc::new(NoPackets(fake.clone()));
    let finished = run(job(
        Endpoint::Remote(handle),
        Endpoint::Local,
        &dest,
        &[&remote],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 6);
    for index in 0..6 {
        let downloaded = dest.join(format!("Vault/f{index}.txt"));
        assert_eq!(
            std::fs::read(&downloaded).expect("downloaded"),
            format!("content {index}").as_bytes()
        );
    }
    assert!(
        fake.counters.opens.load(Ordering::SeqCst) >= 6,
        "every file is read alone"
    );
}
