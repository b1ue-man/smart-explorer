//! Bounded application I/O using the mounted Windows pathname, not engine calls.
use super::{MountedOptimization, OptimizationBackend};
use super::super::super::{callbacks_io, dokany_abi::{DokanFileInfo, DokanOperations, NtStatus, OPTION_ALLOW_IPC_BATCHING}};
use std::{
    ffi::c_void,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::Path,
    sync::{mpsc, Arc, Condvar, Mutex, atomic::{AtomicUsize, Ordering}},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, MOVEFILE_REPLACE_EXISTING,
};

#[path = "optimization_script_probe.rs"]
mod script_probe;

const BURST_BYTES: &[u8] = b"bounded parallel mounted-file workload\n";
static READS: AtomicUsize = AtomicUsize::new(0);
static WRITES: AtomicUsize = AtomicUsize::new(0);
static FLUSHES: AtomicUsize = AtomicUsize::new(0);

pub(super) fn seed(backend: &OptimizationBackend) {
    for number in 0..24 { backend.put(&format!("/burst/{number:02}.txt"), BURST_BYTES); }
    script_probe::seed(backend);
}

pub(super) fn exercise(fixture: &mut MountedOptimization) -> io::Result<()> {
    let root = fixture.root()?;
    let backend = Arc::clone(&fixture.backend);
    let output = fixture.temporary.path().join("script-result.txt");
    let private = fixture.storage().options.options & OPTION_ALLOW_IPC_BATCHING != 0;
    let script = script_probe::ScriptProbe::new();
    let runner = script.runner();
    let (send, receive) = mpsc::channel();
    let worker = thread::Builder::new().name("optimization-mounted-app".into()).spawn(move || {
        eprintln!("[mount optimization] private={private} phase=save-workload-begin");
        let result = save_workload(&root, &backend)
            .and_then(|()| {
                eprintln!("[mount optimization] private={private} phase=save-workload-end");
                runner.run(&root, &output, private)
            })
            .and_then(|()| {
                runner.check_active()?;
                eprintln!("[mount optimization] private={private} phase=parallel-burst-begin");
                parallel_burst(&root)
            });
        let _ = send.send(result);
    })?;
    let result = match receive.recv_timeout(Duration::from_secs(120)) {
        Ok(result) => result,
        Err(error) => Err(io::Error::other(format!("mounted app deadline: {error}"))),
    };
    // The coordinator retains the child on script timeout. Request termination
    // without waiting, then release mounted I/O before joining/reaping. Never
    // let a worker's Drop wait for the process before this unmount can happen.
    if result.is_err() {
        script.cancel();
        eprintln!("[mount optimization] private={private} phase=app-error-unmount-begin");
        fixture.close();
        eprintln!("[mount optimization] private={private} phase=app-error-unmount-end");
    }
    let joined = worker.join().map_err(|_| io::Error::other("mounted app worker panicked"));
    script.reap();
    joined?;
    result
}

fn save_workload(root: &Path, backend: &OptimizationBackend) -> io::Result<()> {
    let note = root.join("vault/note.md");
    let mut file = OpenOptions::new().create_new(true).read(true).write(true).open(&note)?;
    file.write_all(b"hello-long-note")?;
    file.sync_all()?;
    assert_eq!(backend.bytes("/vault/note.md"), b"hello-long-note");
    file.set_len(5)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(b"saved")?;
    file.sync_all()?;
    drop(file);
    assert_eq!(backend.bytes("/vault/note.md"), b"saved");
    assert_eq!(fs::read(&note)?, b"saved");

    // Keep both an unmaterialized and a materialized delete-sharing handle.
    let mut old_lazy = shared_read(&note)?;
    let mut old_read = shared_read(&note)?;
    let mut initial = Vec::new();
    old_read.read_to_end(&mut initial)?;
    assert_eq!(initial, b"saved");
    let staged = root.join("vault/note.tmp");
    let mut temp = OpenOptions::new().create_new(true).write(true).open(&staged)?;
    temp.write_all(b"atomic-replacement")?;
    temp.sync_all()?;
    drop(temp);
    replace_file(&staged, &note)?;
    assert_eq!(backend.bytes("/vault/note.md"), b"atomic-replacement");
    assert_eq!(fs::read(&note)?, b"atomic-replacement");
    for old in [&mut old_lazy, &mut old_read] {
        old.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        old.read_to_end(&mut bytes)?;
        assert_eq!(bytes, b"saved", "old handle followed the replacement pathname");
    }
    drop(old_lazy);
    drop(old_read);
    fs::remove_file(&note)?;
    assert!(matches!(fs::metadata(&note), Err(error) if error.kind() == io::ErrorKind::NotFound));
    assert!(matches!(crate::vfs::Backend::stat(backend, "/vault/note.md"),
        Err(error) if error.kind() == io::ErrorKind::NotFound));
    Ok(())
}

fn shared_read(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).open(path)
}

fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    let source = wide(source);
    let destination = wide(destination);
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), MOVEFILE_REPLACE_EXISTING) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct BurstWorkers(Vec<thread::JoinHandle<()>>);
impl Drop for BurstWorkers {
    fn drop(&mut self) {
        for worker in self.0.drain(..) { let _ = worker.join(); }
    }
}

fn parallel_burst(root: &Path) -> io::Result<()> {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let (send, receive) = mpsc::channel();
    let mut workers = BurstWorkers(Vec::new());
    for number in 0..24 {
        let path = root.join(format!("burst/{number:02}.txt"));
        let gate = Arc::clone(&gate);
        let send = send.clone();
        workers.0.push(thread::Builder::new().name(format!("optimization-burst-{number}")).spawn(move || {
            let result = (|| {
                let ready = gate.0.lock().map_err(|_| io::Error::other("burst gate poisoned"))?;
                let (ready, _) = gate.1.wait_timeout_while(ready, Duration::from_secs(5), |ready| !*ready)
                    .map_err(|_| io::Error::other("burst gate poisoned"))?;
                if !*ready { return Err(io::Error::other("burst start timed out")); }
                drop(ready);
                for _ in 0..32 {
                    assert_eq!(fs::metadata(&path)?.len(), BURST_BYTES.len() as u64);
                    assert_eq!(fs::read(&path)?, BURST_BYTES);
                }
                Ok(())
            })();
            let _ = send.send(result);
        })?);
    }
    *gate.0.lock().map_err(|_| io::Error::other("burst gate poisoned"))? = true;
    gate.1.notify_all();
    drop(send);
    let deadline = Instant::now() + Duration::from_secs(60);
    for _ in 0..workers.0.len() {
        receive.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|error| io::Error::other(format!("parallel mounted I/O deadline: {error}")))??;
    }
    drop(workers);
    Ok(())
}

fn wide(path: &Path) -> Vec<u16> { path.as_os_str().encode_wide().chain(Some(0)).collect() }

pub(super) fn install_counters(operations: &mut DokanOperations) {
    for counter in [&READS, &WRITES, &FLUSHES] { counter.store(0, Ordering::Relaxed); }
    operations.read_file = Some(read);
    operations.write_file = Some(write);
    operations.flush_file_buffers = Some(flush);
}

pub(super) fn assert_callbacks() {
    for (name, count) in [("read", &READS), ("write", &WRITES), ("flush", &FLUSHES)] {
        let count = count.load(Ordering::Relaxed);
        assert!(count > 0, "actual Windows {name} never reached production callback");
        eprintln!("[mount optimization] {name}_callbacks={count}");
    }
}

unsafe extern "system" fn read(name: *const u16, buffer: *mut c_void, length: u32,
    transferred: *mut u32, offset: i64, info: *mut DokanFileInfo) -> NtStatus {
    READS.fetch_add(1, Ordering::Relaxed);
    unsafe { callbacks_io::read_file(name, buffer, length, transferred, offset, info) }
}
unsafe extern "system" fn write(name: *const u16, buffer: *const c_void, length: u32,
    transferred: *mut u32, offset: i64, info: *mut DokanFileInfo) -> NtStatus {
    WRITES.fetch_add(1, Ordering::Relaxed);
    unsafe { callbacks_io::write_file(name, buffer, length, transferred, offset, info) }
}
unsafe extern "system" fn flush(name: *const u16, info: *mut DokanFileInfo) -> NtStatus {
    FLUSHES.fetch_add(1, Ordering::Relaxed);
    unsafe { callbacks_io::flush_file_buffers(name, info) }
}
