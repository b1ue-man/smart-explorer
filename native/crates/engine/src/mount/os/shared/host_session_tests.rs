//! Signals of the shared host flow (DRV-M2): the step order of the Windows
//! host of v0.5.182, and the update stop of contract V8 (unmount like
//! `se drive unmount` without `--force`, journal kept, exit 75), observed
//! with a recording platform and control over the real engine.
use super::super::change_pump::callback_path;
use super::super::host_platform::{
    HostLease, HostPlatform, HostStepError, VolumeLocation, VolumePoll,
};
use super::{run, HostControl, HostExit, HostInputs, UpdateStopCheck};
use crate::daemon::{MountHostConfig, UpdateStopBaseline};
use crate::mount::optimization_fixture::{FixtureDirectory, OptimizationBackend};
use crate::mount::{
    BackendRoot, CacheSpaceProbe, DriveLetter, DriveSelection, HandleId, MountConfig, MountEngine,
    MountFilesystem, MountId, MountMode, MountRecovery, MountRuntimeConfig, MountSource,
    MountStatus, OpenDisposition, OpenFileOptions, PathDialect,
};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
fn drive_task_host_session_order_matches_windows_host() {
    let fixture = Fixture::new("order");
    // A marker left by an earlier update must not stop this host.
    *fixture.marker.lock().unwrap() = Some(UNIX_EPOCH + Duration::from_secs(10));
    let check = fixture.check();
    let control = Control::new(&fixture.log, 2, None);
    let mut platform = Platform::new(&fixture, Background::Nothing);
    let result = run(fixture.inputs(), &control, &mut platform, &check);
    assert_eq!(result, Ok(HostExit::Stopped));
    let z = DriveLetter::parse('Z').unwrap();
    let expected = [
        "lease".to_string(),
        "space".into(),
        report(MountStatus::Mounting, Some(MountRecovery::Clean)),
        "runtime".into(),
        report(MountStatus::Mounting, Some(MountRecovery::Clean)),
        "prepare_volume".into(),
        report(MountStatus::Mounting, Some(MountRecovery::Unknown)),
        "start_volume".into(),
        "start_background".into(),
        "location".into(),
        report(MountStatus::Mounted { drive: z }, None),
        "wait 1".into(),
        "stop_requested".into(),
        "poll".into(),
        "deliver".into(),
        "wait 2".into(),
        "shutdown".into(),
        report(MountStatus::Unmounted, Some(MountRecovery::Clean)),
        "release_volume".into(),
        "finish_runtime true".into(),
        "lease released".into(),
    ];
    assert_eq!(fixture.log.steps(), expected);
    // The runtime is loaded before any remote access; the remote root check,
    // the retry and the root preload run before the frontend state exists.
    let stats = platform.stats.lock().unwrap().clone();
    assert_eq!(stats.len(), 2);
    assert_eq!(stats[0], 0, "remote access before the runtime was loaded");
    assert!(stats[1] > stats[0], "no remote check before the volume");
    assert_eq!(HostExit::Stopped.exit_code(), 0);
}

#[test]
fn drive_task_host_session_update_stop_keeps_unpublished_changes() {
    // Publishing works: the update stop unmounts like `se drive unmount`, the
    // change reaches the remote, the journal is clean, the exit code is 75.
    let fixture = Fixture::new("update-published");
    fixture.backend.put("/note", b"old");
    let check = fixture.check();
    let marker = Arc::clone(&fixture.marker);
    let request = Box::new(move |wait: usize| {
        if wait == 1 {
            *marker.lock().unwrap() = Some(SystemTime::now());
        }
    });
    let control = Control::new(&fixture.log, usize::MAX, Some(request));
    let mut platform = Platform::new(&fixture, Background::WriteNote);
    let result = run(fixture.inputs(), &control, &mut platform, &check);
    assert_eq!(result, Ok(HostExit::UpdateStop));
    assert_eq!(result.unwrap().exit_code(), 75);
    assert_eq!(fixture.backend.bytes("/note"), b"new");
    let steps = fixture.log.steps();
    assert_eq!(
        steps
            .iter()
            .rev()
            .take(4)
            .rev()
            .cloned()
            .collect::<Vec<_>>(),
        [
            report(MountStatus::Unmounted, Some(MountRecovery::Clean)),
            "release_volume".into(),
            "finish_runtime true".into(),
            "lease released".into(),
        ]
    );
    assert_eq!(fixture.recovery(), MountRecovery::Clean);

    // Publishing fails: the same stop keeps the change in the journal and
    // reports it, and the process still ends with 75 for the updater.
    let fixture = Fixture::new("update-kept");
    fixture.backend.put("/note", b"old");
    fixture.backend.fail_upload(true);
    let check = fixture.check();
    let marker = Arc::clone(&fixture.marker);
    let request = Box::new(move |wait: usize| {
        if wait == 1 {
            *marker.lock().unwrap() = Some(SystemTime::now());
        }
    });
    let control = Control::new(&fixture.log, usize::MAX, Some(request));
    let mut platform = Platform::new(&fixture, Background::WriteNote);
    let result = run(fixture.inputs(), &control, &mut platform, &check);
    assert_eq!(result, Ok(HostExit::UpdateStop));
    assert_eq!(fixture.backend.bytes("/note"), b"old");
    let steps = fixture.log.steps();
    let failed = steps
        .iter()
        .find(|step| step.starts_with("report Failed"))
        .expect("the kept change is reported");
    assert!(
        failed.contains("1 cached change(s) remain after unmount"),
        "{failed}"
    );
    assert!(failed.ends_with("Some(Required)"), "{failed}");
    assert!(steps.contains(&"finish_runtime false".to_string()));
    assert_eq!(fixture.recovery(), MountRecovery::Required);
    let journal = fixture.journal();
    assert!(journal.contains("\"remote_path\":\"/note\""), "{journal}");
}

#[test]
fn drive_task_change_pump_maps_backend_paths_per_dialect() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.mkdir("/srv/docs");
    // POSIX keeps every name character Windows refuses.
    let posix = MountEngine::open_host_cache(
        MountRuntimeConfig::new(MountId::parse("pump-posix").unwrap(), MountMode::ReadOnly)
            .with_path_dialect(PathDialect::Posix),
        backend.clone(),
        directory.path(),
    )
    .unwrap();
    let facts = MountFilesystem::volume_facts(&posix);
    assert_eq!(
        callback_path(&facts, &posix, "/srv/docs/a:b").unwrap(),
        "/srv/docs/a:b"
    );
    assert_eq!(callback_path(&facts, &posix, "/").unwrap(), "/");
    // Windows below a root: paths outside it and unrepresentable names are
    // refused, never guessed.
    let config = MountConfig::new(
        MountId::parse("pump-windows").unwrap(),
        MountSource::SavedRemote {
            account: "sftp:pump".into(),
            root: BackendRoot::parse("/srv").unwrap(),
        },
        DriveSelection::Automatic,
        MountMode::ReadOnly,
        "Pump",
    )
    .unwrap();
    let windows = MountEngine::open(config, backend.clone(), directory.path()).unwrap();
    let facts = MountFilesystem::volume_facts(&windows);
    assert_eq!(facts.root.as_str(), "/srv");
    assert_eq!(
        callback_path(&facts, &windows, "/srv/docs").unwrap(),
        "\\docs"
    );
    assert_eq!(callback_path(&facts, &windows, "/srv").unwrap(), "\\");
    for (backend_path, kind) in [
        ("/srv/docs/a:b", io::ErrorKind::InvalidInput),
        ("/other/x", io::ErrorKind::PermissionDenied),
        ("/srvx/y", io::ErrorKind::PermissionDenied),
    ] {
        let error = callback_path(&facts, &windows, backend_path).unwrap_err();
        assert_eq!(error.kind(), kind, "{backend_path}");
    }
}

fn report(status: MountStatus, recovery: Option<MountRecovery>) -> String {
    format!("report {status:?} {recovery:?}")
}

#[derive(Default)]
struct Log(Mutex<Vec<String>>);

impl Log {
    fn push(&self, step: impl Into<String>) {
        self.0.lock().unwrap().push(step.into());
    }

    fn steps(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

struct Fixture {
    directory: FixtureDirectory,
    backend: Arc<OptimizationBackend>,
    log: Arc<Log>,
    marker: Arc<Mutex<Option<SystemTime>>>,
    id: MountId,
}

impl Fixture {
    fn new(id: &str) -> Self {
        Self {
            directory: FixtureDirectory::new(),
            backend: OptimizationBackend::new(),
            log: Arc::default(),
            marker: Arc::default(),
            id: MountId::parse(id).unwrap(),
        }
    }

    /// Reads the fake marker on every loop turn.
    fn check(&self) -> UpdateStopCheck {
        let marker = Arc::clone(&self.marker);
        let baseline = UpdateStopBaseline::with_source(Arc::new(move || *marker.lock().unwrap()));
        UpdateStopCheck::new(baseline, Duration::ZERO)
    }

    fn inputs(&self) -> HostInputs {
        let config = MountConfig::new(
            self.id.clone(),
            MountSource::SavedRemote {
                account: "sftp:host-session".into(),
                root: BackendRoot::parse("/").unwrap(),
            },
            DriveSelection::Automatic,
            MountMode::ReadWrite,
            "Host session",
        )
        .unwrap();
        HostInputs {
            config: MountHostConfig::from(&config),
            backend: self.backend.clone(),
            cache_root: self.directory.path().join("cache"),
        }
    }

    fn journal(&self) -> String {
        let path = self
            .directory
            .path()
            .join("cache")
            .join(self.id.as_str())
            .join("journal.jsonl");
        std::fs::read_to_string(path).unwrap_or_default()
    }

    /// The recovery state the journal replays to after the host ended.
    fn recovery(&self) -> MountRecovery {
        let cache = self.directory.path().join("cache");
        crate::mount::spool::audit_recovery(&cache, &self.id).unwrap()
    }
}

type WaitHook = Box<dyn Fn(usize) + Send + Sync>;

/// Records reports and waits; the `stop_at`-th wait is the unmount request.
struct Control {
    log: Arc<Log>,
    stop_at: usize,
    waits: AtomicUsize,
    on_wait: Option<WaitHook>,
}

impl Control {
    fn new(log: &Arc<Log>, stop_at: usize, on_wait: Option<WaitHook>) -> Self {
        Self {
            log: Arc::clone(log),
            stop_at,
            waits: AtomicUsize::new(0),
            on_wait,
        }
    }
}

impl HostControl for Control {
    fn report(&self, status: MountStatus, recovery: Option<MountRecovery>) -> Result<(), String> {
        self.log.push(report(status, recovery));
        Ok(())
    }

    fn wait_for_stop(&self, _timeout: Duration) -> bool {
        let wait = self.waits.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(wait < 1_000, "the host never stopped");
        self.log.push(format!("wait {wait}"));
        if let Some(hook) = &self.on_wait {
            hook(wait);
        }
        wait >= self.stop_at
    }
}

/// What the fake frontend does once its volume runs.
enum Background {
    Nothing,
    /// Writes `/note` through the engine and keeps the handle open, like an
    /// editor that saved without closing.
    WriteNote,
}

struct Platform {
    log: Arc<Log>,
    backend: Arc<OptimizationBackend>,
    background: Background,
    engine: Option<Arc<MountEngine>>,
    handles: Vec<HandleId>,
    stats: Mutex<Vec<usize>>,
}

impl Platform {
    fn new(fixture: &Fixture, background: Background) -> Self {
        Self {
            log: Arc::clone(&fixture.log),
            backend: Arc::clone(&fixture.backend),
            background,
            engine: None,
            handles: Vec::new(),
            stats: Mutex::default(),
        }
    }

    fn engine(&self) -> &Arc<MountEngine> {
        self.engine.as_ref().expect("prepared volume")
    }
}

struct LeaseGuard(Arc<Log>);

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        self.0.push("lease released");
    }
}

struct UnlimitedSpace;

impl CacheSpaceProbe for UnlimitedSpace {
    fn available_bytes(&self) -> io::Result<u64> {
        Ok(u64::MAX / 4)
    }
}

impl HostPlatform for Platform {
    fn path_dialect(&self) -> PathDialect {
        PathDialect::Windows
    }

    fn acquire_lease(&mut self, _spool_root: &Path, _id: &MountId) -> io::Result<HostLease> {
        self.log.push("lease");
        Ok(Box::new(LeaseGuard(Arc::clone(&self.log))))
    }

    fn space_probe(&mut self, _spool_root: &Path) -> io::Result<Arc<dyn CacheSpaceProbe>> {
        self.log.push("space");
        Ok(Arc::new(UnlimitedSpace))
    }

    fn select_runtime(
        &mut self,
        _spool_root: &Path,
        _config: &MountHostConfig,
    ) -> Result<(), String> {
        self.log.push("runtime");
        self.stats.lock().unwrap().push(self.backend.stat_count());
        Ok(())
    }

    fn prepare_volume(
        &mut self,
        engine: &Arc<MountEngine>,
        config: &MountHostConfig,
        _spool_root: &Path,
    ) -> Result<(), HostStepError> {
        self.log.push("prepare_volume");
        self.stats.lock().unwrap().push(self.backend.stat_count());
        assert_eq!(engine.config().id, config.id);
        self.engine = Some(Arc::clone(engine));
        Ok(())
    }

    fn start_volume(&mut self) -> Result<(), String> {
        self.log.push("start_volume");
        Ok(())
    }

    fn start_background(&mut self) -> io::Result<()> {
        self.log.push("start_background");
        if let Background::WriteNote = self.background {
            let engine = Arc::clone(self.engine());
            let handle = engine.open_file(
                "\\note",
                OpenFileOptions {
                    writable: true,
                    disposition: OpenDisposition::OpenExisting,
                },
            )?;
            engine.write(handle, 0, b"new")?;
            self.handles.push(handle);
        }
        Ok(())
    }

    fn location(&self) -> io::Result<VolumeLocation> {
        self.log.push("location");
        Ok(VolumeLocation::Drive(DriveLetter::parse('Z')?))
    }

    fn stop_requested(&self) -> bool {
        self.log.push("stop_requested");
        false
    }

    fn poll_volume(&mut self) -> VolumePoll {
        self.log.push("poll");
        VolumePoll::Running
    }

    fn deliver_changes(&mut self) -> io::Result<()> {
        self.log.push("deliver");
        Ok(())
    }

    fn shutdown(&mut self) -> bool {
        self.log.push("shutdown");
        // The frontend finalizes its open handles while it closes: it saves
        // what it can and closes; a failed save stays in the journal.
        if let Some(engine) = self.engine.clone() {
            for handle in self.handles.drain(..) {
                let _ = engine.flush(handle);
                let _ = engine.close(handle);
            }
        }
        true
    }

    fn release_volume(&mut self) {
        if self.engine.take().is_some() {
            self.log.push("release_volume");
        }
    }

    fn finish_runtime(&mut self, completed: bool) {
        self.log.push(format!("finish_runtime {completed}"));
    }
}
