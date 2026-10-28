//! Upload packets against fake peers: refused members and slow wires.
use super::super::test_backend::{job, run, write, Fake};
use crate::transfer::Endpoint;
use crate::vfs::{
    unpublished_error, Backend, BackendHandle, BatchLimits, BatchPut, BatchPutOutcome, Scheme,
    VfsMeta, VfsResult,
};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Forwards the listed methods to the wrapped fake remote.
macro_rules! forward {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
        $(fn $name(&self, $($arg: $ty),*) -> $ret { self.0.$name($($arg),*) })*
    };
}

pub(in super::super) use forward;

/// A packet peer whose first packet refuses its first member before
/// publishing anything; the rest of the fake behaves as usual.
struct Refusing(Arc<Fake>, AtomicBool);

impl Backend for Refusing {
    forward! {
        scheme() -> Scheme; root_display() -> String; namespace_identity() -> String;
        list_dir(path: &str) -> VfsResult<Vec<VfsMeta>>; stat(path: &str) -> VfsResult<VfsMeta>;
        open_read(path: &str) -> VfsResult<Box<dyn Read + Send>>;
        open_write(path: &str) -> VfsResult<Box<dyn Write + Send>>;
        open_write_new(path: &str) -> VfsResult<Box<dyn Write + Send>>;
        open_write_fresh(path: &str, size: u64) -> VfsResult<Option<Box<dyn Write + Send>>>;
        promote_copy_stage(stage: &str, destination: &str) -> VfsResult<()>;
        rename(src: &str, dst: &str) -> VfsResult<()>;
        rename_no_replace(src: &str, dst: &str) -> VfsResult<()>;
        remove_file(path: &str) -> VfsResult<()>; remove_dir(path: &str) -> VfsResult<()>;
        mkdir_all(path: &str) -> VfsResult<()>; create_dir(path: &str) -> VfsResult<()>;
        create_dir_new(path: &str) -> VfsResult<()>;
        discard_copy_stage(stage: &str) -> VfsResult<()>;
        case_sensitive_paths(root: &str) -> bool; flow_key(path: &str) -> String;
        concurrent_read_write() -> bool; batch_limits(dir: &str) -> Option<BatchLimits>;
    }

    fn put_batch(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
    ) -> VfsResult<Vec<BatchPutOutcome>> {
        let Some((first, rest)) = entries.split_first() else {
            return Ok(Vec::new());
        };
        if self.1.swap(true, Ordering::SeqCst) {
            return self.0.put_batch(entries, data);
        }
        data.read_exact(&mut vec![0u8; first.size as usize])?;
        let refused = unpublished_error(io::ErrorKind::ConnectionReset, "fixture: refused");
        let mut outcomes = vec![BatchPutOutcome::Failed(refused)];
        outcomes.extend(self.0.put_batch(rest, data)?);
        Ok(outcomes)
    }
}

#[test]
fn speed_task_batch_unpublished_failure_is_retried_once() {
    let root = tempfile::tempdir().expect("temp dir");
    let local = root.path().join("local/Vault");
    for index in 0..4 {
        write(
            &local.join(format!("f{index}.txt")),
            format!("file {index}").as_bytes(),
        );
    }
    let remote = root.path().join("remote");
    std::fs::create_dir(&remote).expect("remote");
    let mut fake = Fake::new("unpublished-retry");
    fake.batch = Some(BatchLimits {
        max_files: 64,
        max_bytes: 1024 * 1024,
    });
    let (fake, _) = fake.handle();
    let handle: BackendHandle = Arc::new(Refusing(fake.clone(), AtomicBool::new(false)));
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&local],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 4);
    for index in 0..4 {
        let published = remote.join(format!("Vault/f{index}.txt"));
        assert_eq!(
            std::fs::read(&published).expect("published"),
            format!("file {index}").as_bytes()
        );
    }
    // Exactly one single upload: the refused member, sent once more.
    assert_eq!(fake.counters.stages.load(Ordering::SeqCst), 1);
}

/// A packet peer far away: every packet takes a while on the wire.
struct SlowWire(Arc<Fake>);

impl Backend for SlowWire {
    forward! {
        scheme() -> Scheme; root_display() -> String; namespace_identity() -> String;
        list_dir(path: &str) -> VfsResult<Vec<VfsMeta>>; stat(path: &str) -> VfsResult<VfsMeta>;
        open_read(path: &str) -> VfsResult<Box<dyn Read + Send>>;
        open_write(path: &str) -> VfsResult<Box<dyn Write + Send>>;
        open_write_new(path: &str) -> VfsResult<Box<dyn Write + Send>>;
        open_write_fresh(path: &str, size: u64) -> VfsResult<Option<Box<dyn Write + Send>>>;
        promote_copy_stage(stage: &str, destination: &str) -> VfsResult<()>;
        rename(src: &str, dst: &str) -> VfsResult<()>;
        rename_no_replace(src: &str, dst: &str) -> VfsResult<()>;
        remove_file(path: &str) -> VfsResult<()>; remove_dir(path: &str) -> VfsResult<()>;
        mkdir_all(path: &str) -> VfsResult<()>; create_dir(path: &str) -> VfsResult<()>;
        create_dir_new(path: &str) -> VfsResult<()>;
        discard_copy_stage(stage: &str) -> VfsResult<()>;
        case_sensitive_paths(root: &str) -> bool; flow_key(path: &str) -> String;
        transfer_ceiling(path: &str) -> Option<usize>;
        concurrent_read_write() -> bool; batch_limits(dir: &str) -> Option<BatchLimits>;
    }

    fn put_batch(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
    ) -> VfsResult<Vec<BatchPutOutcome>> {
        std::thread::sleep(std::time::Duration::from_millis(300));
        self.0.put_batch(entries, data)
    }
}

#[test]
fn speed_task_batch_jobs_book_time_accounts() {
    // 200 tiny files in packets of 8, each packet 300 ms on the wire: the
    // time goes to the target side, and after three windows the progress
    // names the connection, which only packets-with-accounts can do
    let root = tempfile::tempdir().expect("temp dir");
    let local = root.path().join("local/Vault");
    for index in 0..200 {
        write(&local.join(format!("f{index:03}.txt")), b"small file");
    }
    let remote = root.path().join("remote");
    std::fs::create_dir(&remote).expect("remote");
    let mut fake = Fake::new("packet-accounts");
    fake.ceiling = Some(1);
    fake.batch = Some(BatchLimits {
        max_files: 8,
        max_bytes: 1024 * 1024,
    });
    let (fake, _) = fake.handle();
    let handle: BackendHandle = Arc::new(SlowWire(fake.clone()));
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&local],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 200);
    assert!(fake.counters.put_batches.load(Ordering::SeqCst) >= 20);
    let progress = &finished.progress;
    assert!(
        progress.limiter.names_a_resource(),
        "{:?} {:?}",
        progress.limiter,
        progress.limiter_text
    );
    let [source, target, _, _] = progress.limiter_shares;
    assert!(target > source, "{:?}", progress.limiter_shares);
}
