//! Actual-driver portion of the single remote mount-optimization task suite.
use super::*;
use super::super::{callback_reporter::CallbackReporter, dokany_abi::OPTION_ALLOW_IPC_BATCHING};
use crate::mount::optimization_fixture::OptimizationBackend;
use std::{
    fs,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread::{self, JoinHandle},
};

#[path = "optimization_volume_io.rs"]
mod volume_io;
#[path = "optimization_watch_tests.rs"]
mod watch;

struct MountedOptimization {
    filesystem: Option<DokanyFileSystem>,
    storage: Option<CallbackStorage>,
    statuses: Receiver<MountStatus>,
    backend: Arc<OptimizationBackend>,
    lease: Option<CacheLease>,
    spool: PathBuf,
    id: MountId,
    temporary: tempfile::TempDir,
}

impl MountedOptimization {
    fn start(private: bool) -> io::Result<Self> {
        let temporary = tempfile::tempdir()?;
        let spool = prepare_spool_root(&temporary.path().join("spool"))?;
        let id = MountId::new_random()?;
        let lease = CacheLease::acquire(&spool, &id)?;
        let backend = OptimizationBackend::new();
        for directory in ["/watch", "/vault", "/scripts", "/burst"] {
            backend.mkdir(directory);
        }
        volume_io::seed(&backend);
        let handle: crate::vfs::BackendHandle = backend.clone();
        let engine = Arc::new(MountEngine::open_host_cache(
            MountRuntimeConfig::new(id.clone(), MountMode::ReadWrite), handle, &spool,
        )?.with_cache_space_probe(Arc::new(super::super::cache_space::CacheDiskSpace::new(&spool)?)));
        engine.prepare_host_remote()?;
        engine.retry_pending_changes()?;
        engine.preload_metadata()?;
        // Never allow the private acceptance case to succeed via fallback.
        let runtime = if private {
            DokanyRuntime::preflight_private(&spool)
        } else {
            DokanyRuntime::preflight()
        }.map_err(|error| io::Error::other(format!("runtime private={private}: {error}")))?;
        assert_eq!(runtime.is_private(), private);
        eprintln!("[mount optimization] private={private} path={} versions={:?}",
            runtime.loaded_path()?.display(), runtime.info());
        let candidates = drive_candidates(DriveSelection::Automatic).map_err(io::Error::other)?;
        let initial = *candidates.first().ok_or_else(|| io::Error::other("no unused drive letter"))?;
        let (send, statuses) = mpsc::channel();
        let context = Box::new(CallbackContext::new(
            engine, runtime.clone(), CallbackReporter::Capture(send), initial, false,
            "Mount optimization task".into(), super::super::metadata::volume_serial(id.as_str()),
            absolute_path_wide(&spool)?,
        )?);
        let mut storage = CallbackStorage::new(context, false);
        volume_io::install_counters(&mut storage.operations);
        // Official creation must clear even deliberately reused batching flags.
        storage.options.options |= OPTION_ALLOW_IPC_BATCHING;
        let filesystem = start_on_available_drive(&runtime, &mut storage, &candidates)
            .map_err(io::Error::other)?;
        assert_eq!(storage.options.single_thread, 0);
        assert_eq!(storage.options.options & OPTION_ALLOW_IPC_BATCHING != 0, private);
        let fixture = Self {
            filesystem: Some(filesystem), storage: Some(storage), statuses, backend,
            lease: Some(lease), spool, id, temporary,
        };
        fixture.storage().start_metadata_refresh()?;
        match fixture.statuses.recv_timeout(Duration::from_secs(5)) {
            Ok(MountStatus::Mounted { drive }) if drive == fixture.drive()? => {}
            other => return Err(io::Error::other(format!("Mounted callback absent: {other:?}"))),
        }
        Ok(fixture)
    }

    fn storage(&self) -> &CallbackStorage { self.storage.as_ref().expect("live callback storage") }
    fn filesystem(&self) -> &DokanyFileSystem { self.filesystem.as_ref().expect("live filesystem") }
    fn drive(&self) -> io::Result<DriveLetter> { self.storage().context.selected_drive() }
    fn root(&self) -> io::Result<PathBuf> { Ok(PathBuf::from(format!("{}:\\", self.drive()?.get()))) }

    fn healthy(&self) -> io::Result<()> {
        if self.storage().context.stop_requested() {
            return Err(io::Error::other("production callback supervisor requested stop"));
        }
        for status in self.statuses.try_iter() {
            if matches!(status, MountStatus::Failed { .. } | MountStatus::Conflict { .. }
                | MountStatus::RuntimeUnavailable { .. }) {
                return Err(io::Error::other(format!("production callback status: {status:?}")));
            }
        }
        Ok(())
    }

    fn close(&mut self) {
        if let Some(storage) = self.storage.as_ref() { storage.request_metadata_refresh_stop(); }
        if let Some(filesystem) = self.filesystem.take() { filesystem.close(); }
        if let Some(storage) = self.storage.as_ref() { storage.join_metadata_refresh(); }
    }

    fn finish(mut self) -> io::Result<()> {
        let drive = self.drive()?;
        self.close();
        self.healthy()?;
        assert!(self.storage().context.engine.dirty_entries()?.is_empty(), "dirty entries after teardown");
        volume_io::assert_callbacks();
        drop(self.storage.take());
        drop(self.lease.take());
        assert!(super::super::cache_lease::audit_recovery(&self.spool, &self.id)?.is_clean(),
            "actual journal replay requires recovery after successful workload");
        assert_eq!(fs::read_dir(self.spool.join(self.id.as_str()).join("files"))?.count(), 0,
            "disposable spools survived engine teardown");
        assert_eq!(unsafe { GetLogicalDrives() } & (1 << (drive.get() as u8 - b'A')), 0,
            "mount drive remained registered after DokanCloseHandle");
        Ok(())
    }
}

impl Drop for MountedOptimization {
    fn drop(&mut self) { self.close(); }
}

/// A stuck kernel request must terminate this test process, not leak threads,
/// reuse its drive, or proceed to the second runtime with a possibly live mount.
struct Deadline {
    cancel: mpsc::Sender<()>,
    worker: Option<JoinHandle<()>>,
}

impl Deadline {
    fn start() -> io::Result<Self> {
        let (cancel, wait) = mpsc::channel();
        let worker = thread::Builder::new().name("optimization-volume-deadline".into()).spawn(move || {
            if matches!(wait.recv_timeout(Duration::from_secs(180)), Err(mpsc::RecvTimeoutError::Timeout)) {
                eprintln!("[mount optimization] fatal 180-second volume deadline; no further mount attempted");
                std::process::abort();
            }
        })?;
        Ok(Self { cancel, worker: Some(worker) })
    }
}

impl Drop for Deadline {
    fn drop(&mut self) {
        let _ = self.cancel.send(());
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

#[test]
#[ignore = "remote Windows suite only: requires approved private payload and installed Dokany driver"]
fn mount_optimization_task_actual_volume_apps_watchers_and_batching() -> io::Result<()> {
    for private in [true, false] {
        let _deadline = Deadline::start()?;
        let mut fixture = MountedOptimization::start(private)?;
        watch::exercise(&fixture)?;
        volume_io::exercise(&mut fixture)?;
        fixture.healthy()?;
        let continuation = fixture.filesystem().batch_continuation_count();
        if private {
            assert!(continuation.is_some_and(|count| count > 0),
                "private workload consumed no second-or-later batched event: {continuation:?}");
        } else {
            assert_eq!(continuation, None);
            assert_eq!(fixture.storage().options.options & OPTION_ALLOW_IPC_BATCHING, 0);
        }
        eprintln!("[mount optimization] private={private} continuation={continuation:?} backend_reads={}",
            fixture.backend.read_count());
        fixture.finish()?;
    }
    // Keep the previously established real-driver navigation/checker regression
    // in this same selected task. Its finite stall is last: no mount starts after it.
    let _deadline = Deadline::start()?;
    super::mount_batching_task::mount_batching_task_real_driver_navigation_and_checker()?;
    Ok(())
}
