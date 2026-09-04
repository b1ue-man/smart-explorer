use super::*;
use super::super::callback_reporter::CallbackReporter;
use super::super::dokany_abi::OPTION_ALLOW_IPC_BATCHING;
use crate::vfs::BackendHandle;
use std::{
    fs::{self, File},
    io::Read,
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::Instant,
};

#[path = "mounted_volume_fixture_backend.rs"]
mod fixture_backend;
use fixture_backend::{FixtureBackend, CONTENTS, DEPTH};

struct MountedFixture {
    filesystem: Option<DokanyFileSystem>,
    storage: CallbackStorage,
    statuses: Receiver<MountStatus>,
    backend: Arc<FixtureBackend>,
    logs: PathBuf,
    _lease: CacheLease,
    temporary: tempfile::TempDir,
}

impl MountedFixture {
    fn start() -> io::Result<Self> {
        let temporary = tempfile::tempdir()?;
        let spool = prepare_spool_root(&temporary.path().join("spool"))?;
        let id = MountId::new_random()?;
        let log_root = PathBuf::from(std::env::var_os("SMART_EXPLORER_MOUNT_TASK_LOG_ROOT")
            .ok_or_else(|| io::Error::other("SMART_EXPLORER_MOUNT_TASK_LOG_ROOT is required"))?);
        if !log_root.is_absolute() || !log_root.is_dir() {
            return Err(io::Error::other("task log root must be an existing absolute directory"));
        }
        let logs = log_root.join(format!("volume-{}", id.as_str()));
        fs::create_dir(&logs)?;
        let lease = CacheLease::acquire(&spool, &id)?;
        let backend = FixtureBackend::new();
        let handle: BackendHandle = backend.clone();
        let engine = Arc::new(MountEngine::open_host_cache(
            MountRuntimeConfig::new(id.clone(), MountMode::ReadWrite), handle, &spool,
        )?);
        engine.prepare_host_remote()?;
        engine.retry_pending_changes()?;
        engine.preload_metadata()?;
        let runtime = DokanyRuntime::preflight().map_err(io::Error::other)?;
        let candidates = drive_candidates(DriveSelection::Automatic).map_err(io::Error::other)?;
        let initial = *candidates.first().ok_or_else(|| io::Error::other("no available drive"))?;
        let (send, statuses) = mpsc::channel();
        let context = Box::new(CallbackContext::new(
            engine, runtime.clone(), CallbackReporter::Capture(send), initial, false,
            "Mount batching task".into(), super::super::metadata::volume_serial(id.as_str()),
            absolute_path_wide(&spool)?,
        )?);
        let mut storage = CallbackStorage::new(context, false);
        assert_eq!(storage.options.single_thread, 0);
        assert_eq!(storage.options.options & OPTION_ALLOW_IPC_BATCHING, 0);
        // Model options reused after Dokany's CPU-count branch mutated them.
        // The real runtime create boundary must repair this before driver start.
        storage.options.options |= OPTION_ALLOW_IPC_BATCHING;
        let filesystem = start_on_available_drive(&runtime, &mut storage, &candidates)
            .map_err(io::Error::other)?;
        let fixture = Self {
            filesystem: Some(filesystem), storage, statuses, backend, logs,
            _lease: lease, temporary,
        };
        fixture.storage.start_metadata_refresh()?;
        match fixture.statuses.recv_timeout(Duration::from_secs(5)) {
            Ok(MountStatus::Mounted { drive }) if drive == fixture.drive()? => {}
            other => return Err(io::Error::other(format!("missing actual mounted status: {other:?}"))),
        }
        Ok(fixture)
    }

    fn drive(&self) -> io::Result<DriveLetter> {
        self.storage.context.selected_drive()
    }

    fn root(&self) -> io::Result<PathBuf> {
        Ok(PathBuf::from(format!("{}:\\", self.drive()?.get())))
    }

    fn close(&mut self) {
        self.backend.release_stall();
        self.storage.request_metadata_refresh_stop();
        if let Some(filesystem) = self.filesystem.take() {
            filesystem.close();
        }
        self.storage.join_metadata_refresh();
    }

    fn assert_healthy(&self) {
        assert!(!self.storage.context.stop_requested(), "callback requested stop");
        for status in self.statuses.try_iter() {
            assert!(!matches!(status, MountStatus::Failed { .. } | MountStatus::Conflict { .. }
                | MountStatus::RuntimeUnavailable { .. }), "callback failure: {status:?}");
        }
        self.backend.assert_read_only();
    }
}

impl Drop for MountedFixture {
    fn drop(&mut self) { self.close(); }
}

#[test]
#[ignore = "requires the pinned System32 Dokany DLL, installed driver and task checker"]
fn mount_batching_task_real_driver_navigation_and_checker() -> io::Result<()> {
    let checker = PathBuf::from(std::env::var_os("SMART_EXPLORER_MOUNT_CHECKER")
        .ok_or_else(|| io::Error::other("SMART_EXPLORER_MOUNT_CHECKER is required"))?);
    if !checker.is_absolute() || !checker.is_file() {
        return Err(io::Error::other("checker must be an existing absolute script path"));
    }
    let mut fixture = MountedFixture::start()?;
    exercise_parallel(&fixture)?;
    let pass = run_checker(&fixture, &checker, "pass", 90, Duration::from_secs(100))?;
    assert_eq!(pass.0.code(), Some(0), "checker did not pass: {}", pass.1);
    assert_eq!(pass.1["outcome"], "PASS");
    assert_eq!(pass.1["read_only_probe"], true);
    assert!(pass.1["surviving_worker_pids"].as_array().expect("survivor results").is_empty());
    let drives = pass.1["drives"].as_array().expect("drive results");
    assert_eq!(drives.len(), 1);
    assert_eq!(drives[0]["drive"], fixture.drive()?.get().to_string());
    let workers = drives[0]["workers"].as_array().expect("worker results");
    assert_eq!(workers.len(), 4);
    for worker in workers {
        assert_eq!(worker["outcome"], "PASS");
        assert_eq!(worker["rounds"], 3);
        assert!(worker["unique_directories"].as_u64().unwrap_or(0) >= 5);
        assert!(worker["files_read"].as_u64().unwrap_or(0) >= 1);
        assert!(worker["bytes_read"].as_u64().unwrap_or(0) > 0);
    }
    fixture.assert_healthy();

    // A fresh root lookup must reach the backend rather than the warm cache.
    // The guard and fixture Drop both release the finite stall before unmount.
    let release = fixture.backend.arm_stall();
    fixture.storage.context.engine.invalidate_metadata("/", true);
    let started = Instant::now();
    let timed_out = run_checker(&fixture, &checker, "timeout", 20, Duration::from_secs(30));
    drop(release);
    let (status, report) = timed_out?;
    assert!(started.elapsed() < Duration::from_secs(31));
    assert!(fixture.backend.stalled_calls() > 0, "timeout never reached backend latch");
    assert_eq!(status.code(), Some(4), "checker did not time out: {report}");
    assert_eq!(report["outcome"], "TIMEOUT");
    fixture.close();
    fixture.assert_healthy();
    assert!(fixture.storage.context.engine.dirty_entries()?.is_empty());
    Ok(())
}

fn exercise_parallel(fixture: &MountedFixture) -> io::Result<()> {
    const WORKERS: usize = 12;
    let root = fixture.root()?;
    let start = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let (send, receive) = mpsc::channel();
    let mut threads = Vec::new();
    for number in 0..WORKERS {
        let root = root.clone();
        let backend = Arc::clone(&fixture.backend);
        let start = Arc::clone(&start);
        let send = send.clone();
        threads.push(thread::Builder::new().name(format!("mounted-reader-{number}"))
            .spawn(move || {
                let result = await_start(&start).and_then(|()| read_tree(&root, &backend, number));
                let _ = send.send(result);
            })?);
    }
    *start.0.lock().map_err(|_| io::Error::other("worker gate poisoned"))? = true;
    start.1.notify_all();
    drop(send);
    let deadline = Instant::now() + Duration::from_secs(60);
    for _ in 0..WORKERS {
        receive.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|error| io::Error::other(format!("mounted worker did not complete: {error}")))??;
    }
    for worker in threads {
        worker.join().map_err(|_| io::Error::other("mounted worker panicked"))?;
    }
    fixture.assert_healthy();
    Ok(())
}

fn await_start(start: &(Mutex<bool>, std::sync::Condvar)) -> io::Result<()> {
    let ready = start.0.lock().map_err(|_| io::Error::other("worker gate poisoned"))?;
    let (ready, _) = start.1.wait_timeout_while(ready, Duration::from_secs(10), |ready| !*ready)
        .map_err(|_| io::Error::other("worker gate poisoned"))?;
    if !*ready {
        return Err(io::Error::new(io::ErrorKind::TimedOut, "workers were not all started"));
    }
    Ok(())
}

fn read_tree(root: &Path, backend: &FixtureBackend, worker: usize) -> io::Result<()> {
    for _ in 0..3 {
        assert_directory(root, "/", backend)?;
        assert_file(&root.join("root.txt"))?;
        let sibling = format!("folder{:02}", worker % 8);
        assert_directory(&root.join(&sibling), &format!("/{sibling}"), backend)?;
        assert_file(&root.join(&sibling).join("note.txt"))?;
        let mut path = root.join("deep");
        let mut virtual_path = "/deep".to_string();
        for depth in 0..=DEPTH {
            if depth > 0 {
                let component = format!("level{depth:02}");
                path.push(&component);
                virtual_path.push('/');
                virtual_path.push_str(&component);
            }
            assert_directory(&path, &virtual_path, backend)?;
            assert_file(&path.join("note.txt"))?;
        }
    }
    Ok(())
}

fn assert_directory(path: &Path, virtual_path: &str, backend: &FixtureBackend) -> io::Result<()> {
    assert!(fs::metadata(path)?.is_dir());
    let mut names = fs::read_dir(path)?.map(|entry| {
        entry.map(|entry| entry.file_name().to_string_lossy().into_owned())
    }).collect::<io::Result<Vec<_>>>()?;
    names.sort();
    assert_eq!(names, backend.expected_names(virtual_path));
    Ok(())
}

fn assert_file(path: &Path) -> io::Result<()> {
    assert_eq!(fs::metadata(path)?.len(), CONTENTS.len() as u64);
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    assert_eq!(bytes, CONTENTS);
    Ok(())
}

fn run_checker(
    fixture: &MountedFixture, checker: &Path, name: &str, timeout: u32, limit: Duration,
) -> io::Result<(ExitStatus, serde_json::Value)> {
    let report = fixture.logs.join(format!("{name}.json"));
    let system_root = std::env::var_os("SystemRoot")
        .ok_or_else(|| io::Error::other("SystemRoot is required"))?;
    let powershell = PathBuf::from(system_root)
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let stdout = File::create(fixture.logs.join(format!("{name}.stdout")))?;
    let stderr = File::create(fixture.logs.join(format!("{name}.stderr")))?;
    let mut child = CapturedChild(Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(checker).arg("-Drive").arg(fixture.drive()?.get().to_string())
        .arg("-ReportPath").arg(&report).arg("-TimeoutSeconds").arg(timeout.to_string())
        .current_dir(fixture.temporary.path()).stdin(Stdio::null())
        .stdout(Stdio::from(stdout)).stderr(Stdio::from(stderr)).spawn()?);
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.0.try_wait()? {
            let bytes = fs::read(&report)?;
            return Ok((status, serde_json::from_slice(&bytes).map_err(io::Error::other)?));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "checker parent deadline"));
        }
        thread::sleep(Duration::from_millis(40));
    }
}

struct CapturedChild(Child);

impl Drop for CapturedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            // A kernel-blocked child may not terminate immediately. Never turn
            // the outer supervisor into an unbounded wait; fixture Drop unmounts.
            let _ = self.0.try_wait();
        }
    }
}
