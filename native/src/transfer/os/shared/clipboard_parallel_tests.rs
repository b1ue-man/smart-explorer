//! Folder staging for other programs: all files arrive, the first error
//! stops the rest and is returned, the flow's limit bounds the concurrency,
//! and a refused thread leaves the calling thread to do the work.
use super::clipboard_parallel::{
    download_files_parallel, download_files_with, Hooks, CLIPBOARD_JOB,
};
use super::entries::RemoteFileEntry;
use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::Duration;

#[derive(Default)]
struct Counters {
    active: AtomicUsize,
    peak: AtomicUsize,
    opens: AtomicUsize,
    threads: Mutex<Vec<ThreadId>>,
}

/// A remote whose files are generated from their path; opening one counts
/// the calls running at the same time and remembers the calling thread.
struct Dummy {
    key: String,
    ceiling: Option<usize>,
    delay: Duration,
    fail_src: Option<String>,
    counters: Arc<Counters>,
}

impl Dummy {
    fn new(ceiling: Option<usize>, delay_ms: u64, fail_src: Option<&str>) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self {
            key: format!(
                "clipboard-parallel-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ),
            ceiling,
            delay: Duration::from_millis(delay_ms),
            fail_src: fail_src.map(str::to_string),
            counters: Arc::new(Counters::default()),
        }
    }
}

fn content(src: &str) -> Vec<u8> {
    (0..1000 + src.len() * 13)
        .map(|index| (index + src.len()) as u8)
        .collect()
}

struct SlowReader {
    inner: Cursor<Vec<u8>>,
    counters: Arc<Counters>,
}

impl Read for SlowReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl Drop for SlowReader {
    fn drop(&mut self) {
        self.counters.active.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The methods the staging never calls.
macro_rules! unsupported_methods {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
        $(fn $name(&self, $($arg: $ty),*) -> $ret {
            Err(io::Error::new(io::ErrorKind::Unsupported, "not used"))
        })*
    };
}

impl Backend for Dummy {
    fn scheme(&self) -> Scheme {
        Scheme::Local
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.counters.opens.fetch_add(1, Ordering::SeqCst);
        self.counters
            .threads
            .lock()
            .unwrap()
            .push(std::thread::current().id());
        if self.fail_src.as_deref() == Some(path) {
            return Err(io::Error::other(format!("boom: {path}")));
        }
        let now = self.counters.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.counters.peak.fetch_max(now, Ordering::SeqCst);
        let reader = Box::new(SlowReader {
            inner: Cursor::new(content(path)),
            counters: self.counters.clone(),
        });
        std::thread::sleep(self.delay);
        Ok(reader)
    }
    unsupported_methods! {
        list_dir(_path: &str) -> VfsResult<Vec<VfsMeta>>;
        stat(_path: &str) -> VfsResult<VfsMeta>;
        open_write(_path: &str) -> VfsResult<Box<dyn Write + Send>>;
        rename(_src: &str, _dst: &str) -> VfsResult<()>;
        remove_file(_path: &str) -> VfsResult<()>;
        remove_dir(_path: &str) -> VfsResult<()>;
        mkdir_all(_path: &str) -> VfsResult<()>;
    }
    fn flow_key(&self, _path: &str) -> String {
        self.key.clone()
    }
    fn flow_identity(&self, _path: &str) -> String {
        self.key.clone()
    }
    fn transfer_ceiling(&self, _path: &str) -> Option<usize> {
        self.ceiling
    }
}

struct Folder(PathBuf);

impl Folder {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "se-clipboard-parallel-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&path).expect("test folder");
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn entries(count: usize) -> Vec<RemoteFileEntry> {
    (0..count)
        .map(|index| {
            let src = format!("/f{index}");
            RemoteFileEntry {
                size: content(&src).len() as u64,
                rel: format!("d{}/f{index}.bin", index % 3),
                src,
            }
        })
        .collect()
}

fn assert_in_place(files: &[RemoteFileEntry], dir: &Path) {
    for file in files {
        let path = dir.join(file.rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        assert_eq!(
            std::fs::read(&path).unwrap_or_default(),
            content(&file.src),
            "{}",
            file.rel
        );
    }
}

fn fixture(
    count: usize,
    ceiling: Option<usize>,
    delay_ms: u64,
    fail: Option<&str>,
) -> (Dummy, Folder, Vec<RemoteFileEntry>) {
    let (folder, files) = (Folder::new(), entries(count));
    for file in &files {
        let path = folder
            .0
            .join(file.rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        std::fs::create_dir_all(path.parent().expect("parent")).expect("folder");
    }
    (Dummy::new(ceiling, delay_ms, fail), folder, files)
}

#[test]
fn speed_task_clipboard_parallel_downloads_all_files_into_place() {
    assert_eq!(CLIPBOARD_JOB, 1 << 47);
    let (backend, folder, files) = fixture(12, None, 5, None);
    download_files_parallel(&backend, &files, &folder.0).expect("download");
    assert_in_place(&files, &folder.0);
    assert_eq!(backend.counters.opens.load(Ordering::SeqCst), 12);
}

#[test]
fn speed_task_clipboard_parallel_first_error_aborts_and_is_returned() {
    let (backend, folder, files) = fixture(30, None, 40, Some("/f0"));
    let error = download_files_parallel(&backend, &files, &folder.0).expect_err("fails");
    assert!(error.contains("boom: /f0"), "{error}");
    // The files after the failure were never started.
    assert!(backend.counters.opens.load(Ordering::SeqCst) < files.len());
}

#[test]
fn speed_task_clipboard_parallel_runs_files_concurrently_up_to_the_flow_limit() {
    // A fresh flow starts at two operations; the backend bound is four.
    let (backend, folder, files) = fixture(8, Some(4), 150, None);
    download_files_parallel(&backend, &files, &folder.0).expect("download");
    assert_in_place(&files, &folder.0);
    let peak = backend.counters.peak.load(Ordering::SeqCst);
    assert!((2..=4).contains(&peak), "peak {peak}");
}

#[test]
fn speed_task_clipboard_parallel_spawn_failure_degrades_to_serial() {
    let me = std::thread::current().id();
    // The system refuses every thread: the calling thread does it all, one
    // file after the other.
    let (backend, folder, files) = fixture(6, None, 5, None);
    let refuse = |_: usize| false;
    download_files_with(&backend, &files, &folder.0, &Hooks { may_spawn: &refuse })
        .expect("download");
    assert_in_place(&files, &folder.0);
    assert_eq!(backend.counters.peak.load(Ordering::SeqCst), 1);
    let threads = backend.counters.threads.lock().unwrap();
    assert!(threads.iter().all(|id| *id == me));
}

#[test]
fn speed_task_clipboard_parallel_single_file_runs_inline() {
    let me = std::thread::current().id();
    let (backend, folder, files) = fixture(1, None, 1, None);
    let asked = AtomicUsize::new(0);
    let count = |_: usize| asked.fetch_add(1, Ordering::SeqCst) < usize::MAX;
    download_files_with(&backend, &files, &folder.0, &Hooks { may_spawn: &count })
        .expect("download");
    assert_in_place(&files, &folder.0);
    assert_eq!(asked.load(Ordering::SeqCst), 0);
    assert_eq!(*backend.counters.threads.lock().unwrap(), vec![me]);
}
