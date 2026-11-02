//! Windows signals of DRV-M2 (plan 2026-10-17 „Rechte, se Desktop,
//! Tresore“): the status table covers every Win32 code se passes to Dokany
//! (no driver needed), and three real-driver cases through the production
//! host flow (`host_session::run` with `DokanyHost`): an unmapped engine
//! error no longer reads as "Access denied", an unreachable host reads as
//! 1232, and an update stop unmounts and ends with exit code 75 (V8).
//! The real-driver cases need the pinned Dokany runtime and run only in the
//! Windows stage of the task suite (`--include-ignored`).
use super::super::super::shared::host_session::{
    self, HostControl, HostExit, HostInputs, UpdateStopCheck,
};
use super::super::callback_reporter::CallbackReporter;
use super::super::callback_status::dokany_status_table::{dokany_maps_win32, DOKANY_MAPPED_WIN32};
use super::super::callback_status::{nt_status_for_io, win32_for_kind};
use super::super::NtStatus;
use super::DokanyHost;
use crate::daemon::{MountHostConfig, UpdateStopBaseline, UPDATE_STOP_POLL};
use crate::mount::optimization_fixture::OptimizationBackend;
use crate::mount::{
    BackendRoot, DriveLetter, DriveSelection, MountConfig, MountId, MountMode, MountRecovery,
    MountSource, MountStatus,
};
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta, VfsResult};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use windows_sys::Win32::Foundation::{
    ERROR_DIRECTORY, ERROR_INVALID_DATA, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER,
    ERROR_NOT_SUPPORTED, STATUS_IO_DEVICE_ERROR,
};

/// Every stable `io::ErrorKind` of the toolchain the batch builds with.
const ALL_KINDS: &[io::ErrorKind] = &[
    io::ErrorKind::NotFound,
    io::ErrorKind::PermissionDenied,
    io::ErrorKind::ConnectionRefused,
    io::ErrorKind::ConnectionReset,
    io::ErrorKind::HostUnreachable,
    io::ErrorKind::NetworkUnreachable,
    io::ErrorKind::ConnectionAborted,
    io::ErrorKind::NotConnected,
    io::ErrorKind::AddrInUse,
    io::ErrorKind::AddrNotAvailable,
    io::ErrorKind::NetworkDown,
    io::ErrorKind::BrokenPipe,
    io::ErrorKind::AlreadyExists,
    io::ErrorKind::WouldBlock,
    io::ErrorKind::NotADirectory,
    io::ErrorKind::IsADirectory,
    io::ErrorKind::DirectoryNotEmpty,
    io::ErrorKind::ReadOnlyFilesystem,
    io::ErrorKind::StaleNetworkFileHandle,
    io::ErrorKind::InvalidInput,
    io::ErrorKind::InvalidData,
    io::ErrorKind::TimedOut,
    io::ErrorKind::WriteZero,
    io::ErrorKind::StorageFull,
    io::ErrorKind::NotSeekable,
    io::ErrorKind::QuotaExceeded,
    io::ErrorKind::FileTooLarge,
    io::ErrorKind::ResourceBusy,
    io::ErrorKind::ExecutableFileBusy,
    io::ErrorKind::Deadlock,
    io::ErrorKind::CrossesDevices,
    io::ErrorKind::TooManyLinks,
    io::ErrorKind::InvalidFilename,
    io::ErrorKind::ArgumentListTooLong,
    io::ErrorKind::Interrupted,
    io::ErrorKind::Unsupported,
    io::ErrorKind::UnexpectedEof,
    io::ErrorKind::OutOfMemory,
    io::ErrorKind::Other,
];

#[test]
fn drive_task_windows_status_table_covers_emitted_codes() {
    assert_eq!(DOKANY_MAPPED_WIN32.len(), 622);
    assert!(
        DOKANY_MAPPED_WIN32.windows(2).all(|pair| pair[0] < pair[1]),
        "the table must stay sorted and free of duplicates"
    );
    for kind in ALL_KINDS {
        if let Some(code) = win32_for_kind(*kind) {
            assert!(
                dokany_maps_win32(code),
                "{kind:?} maps to {code}, unknown to Dokany"
            );
        }
    }
    // The codes the callbacks pass through `win32(…)`.
    for code in [
        ERROR_DIRECTORY,
        ERROR_NOT_SUPPORTED,
        ERROR_INVALID_PARAMETER,
        ERROR_INVALID_HANDLE,
    ] {
        assert!(dokany_maps_win32(code), "callback code {code}");
    }
    // The three rows of decision C4.
    assert_eq!(win32_for_kind(io::ErrorKind::HostUnreachable), Some(1232));
    assert_eq!(
        win32_for_kind(io::ErrorKind::NetworkUnreachable),
        Some(1231)
    );
    assert_eq!(win32_for_kind(io::ErrorKind::CrossesDevices), Some(17));
    // The former fallback has no Dokany row: it became STATUS_ACCESS_DENIED.
    assert!(!dokany_maps_win32(ERROR_INVALID_DATA));
    let never = |code: u32| -> NtStatus { panic!("{code} must not reach DokanNtStatusFromWin32") };
    for error in [
        io::Error::other("unmapped engine failure"),
        io::Error::new(io::ErrorKind::ReadOnlyFilesystem, "read-only target"),
        io::Error::new(io::ErrorKind::QuotaExceeded, "quota"),
        io::Error::from_raw_os_error(ERROR_INVALID_DATA as i32),
        io::Error::from_raw_os_error(0),
        io::Error::from_raw_os_error(0x7fff_0000),
    ] {
        assert_eq!(
            nt_status_for_io(&error, never),
            STATUS_IO_DEVICE_ERROR,
            "{error}"
        );
    }
    let unreachable = io::Error::new(io::ErrorKind::HostUnreachable, "peer offline");
    let translated = nt_status_for_io(&unreachable, |code| {
        assert_eq!(code, 1232);
        0x0bad_1232
    });
    assert_eq!(translated, 0x0bad_1232);
    let denied = io::Error::new(io::ErrorKind::PermissionDenied, "real denial");
    assert_eq!(nt_status_for_io(&denied, |code| code as NtStatus), 5);
}

#[test]
#[ignore = "requires the pinned Dokany runtime and its installed driver (Windows task stage)"]
fn drive_task_windows_unmapped_error_is_io_device_not_access_denied() -> io::Result<()> {
    let host = MountedHost::start(FailingBackend::new(), MountMode::ReadOnly)?;
    let seen = read_error(&host.root().join("other.txt"))?;
    eprintln!(
        "[drive task] unmapped engine error seen as Win32 {seen:?}; expected 1117 ERROR_IO_DEVICE (assumption A12)"
    );
    let (result, _) = host.finish(false)?;
    assert_eq!(result, Ok(HostExit::Stopped));
    assert!(seen.is_some(), "the failing read reported no Win32 code");
    assert_ne!(
        seen,
        Some(5),
        "an unmapped error still reads as Access denied"
    );
    Ok(())
}

#[test]
#[ignore = "requires the pinned Dokany runtime and its installed driver (Windows task stage)"]
fn drive_task_windows_host_unreachable_maps_1232() -> io::Result<()> {
    let host = MountedHost::start(FailingBackend::new(), MountMode::ReadOnly)?;
    let seen = read_error(&host.root().join("unreachable.txt"))?;
    let readable = std::fs::read(host.root().join("fine.txt"));
    eprintln!("[drive task] unreachable host seen as Win32 {seen:?}");
    let (result, _) = host.finish(false)?;
    assert_eq!(result, Ok(HostExit::Stopped));
    assert_eq!(readable?, b"fine");
    assert_eq!(seen, Some(1232));
    Ok(())
}

#[test]
#[ignore = "requires the pinned Dokany runtime and its installed driver (Windows task stage)"]
fn drive_task_host_session_update_stop_unmounts_and_exits() -> io::Result<()> {
    let backend = OptimizationBackend::new();
    backend.put("/note.txt", b"before");
    let host = MountedHost::start(backend.clone(), MountMode::ReadWrite)?;
    let root = host.root();
    let mut file = File::create(root.join("note.txt"))?;
    file.write_all(b"after the update stop")?;
    drop(file);
    let (result, reports) = host.finish(true)?;
    assert_eq!(result, Ok(HostExit::UpdateStop));
    assert_eq!(HostExit::UpdateStop.exit_code(), 75);
    assert_eq!(backend.bytes("/note.txt"), b"after the update stop");
    assert!(
        !root.exists(),
        "the drive stayed mounted after the update stop"
    );
    assert_eq!(
        reports.last(),
        Some(&(MountStatus::Unmounted, Some(MountRecovery::Clean)))
    );
    Ok(())
}

/// The Win32 code an application sees when it opens and reads `path`.
fn read_error(path: &Path) -> io::Result<Option<i32>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) => return Ok(error.raw_os_error()),
    };
    let mut buffer = [0u8; 16];
    match file.read(&mut buffer) {
        Ok(read) => Err(io::Error::other(format!(
            "reading {} succeeded with {read} bytes",
            path.display()
        ))),
        Err(error) => Ok(error.raw_os_error()),
    }
}

type Report = (MountStatus, Option<MountRecovery>);

/// The daemon side of a test host: reports go to a channel, the unmount
/// request is a flag.
struct ChannelControl {
    reports: mpsc::Sender<Report>,
    stop: Arc<AtomicBool>,
}

impl HostControl for ChannelControl {
    fn report(&self, status: MountStatus, recovery: Option<MountRecovery>) -> Result<(), String> {
        let _ = self.reports.send((status, recovery));
        Ok(())
    }

    fn wait_for_stop(&self, timeout: Duration) -> bool {
        std::thread::sleep(timeout);
        self.stop.load(Ordering::Acquire)
    }
}

/// One production Dokany host on its own thread.
struct MountedHost {
    drive: DriveLetter,
    reports: mpsc::Receiver<Report>,
    /// Kept open: a callback report into a closed channel stops the volume.
    statuses: mpsc::Receiver<MountStatus>,
    done: mpsc::Receiver<Result<HostExit, String>>,
    stop: Arc<AtomicBool>,
    marker: Arc<Mutex<Option<SystemTime>>>,
    _temporary: tempfile::TempDir,
}

impl MountedHost {
    fn start(backend: Arc<impl Backend + 'static>, mode: MountMode) -> io::Result<Self> {
        let temporary = tempfile::tempdir()?;
        let config = MountConfig::new(
            MountId::new_random()?,
            MountSource::SavedRemote {
                account: "sftp:windows-task".into(),
                root: BackendRoot::parse("/")?,
            },
            DriveSelection::Automatic,
            mode,
            "Windows task",
        )?;
        let backend: BackendHandle = backend;
        let inputs = HostInputs {
            config: MountHostConfig::from(&config),
            backend,
            cache_root: temporary.path().join("cache"),
        };
        let (report_send, reports) = mpsc::channel();
        let (status_send, statuses) = mpsc::channel();
        let (done_send, done) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let marker = Arc::new(Mutex::new(None));
        let thread_stop = Arc::clone(&stop);
        let source = Arc::clone(&marker);
        std::thread::Builder::new()
            .name("windows-task-host".into())
            .spawn(move || {
                let baseline = UpdateStopBaseline::with_source(Arc::new(move || {
                    *source.lock().unwrap_or_else(|error| error.into_inner())
                }));
                let check = UpdateStopCheck::new(baseline, UPDATE_STOP_POLL);
                let control = ChannelControl {
                    reports: report_send,
                    stop: thread_stop,
                };
                let mut platform = DokanyHost::new(CallbackReporter::Capture(status_send));
                let _ = done_send.send(host_session::run(inputs, &control, &mut platform, &check));
            })?;
        let mut host = Self {
            drive: DriveLetter::parse('Z')?,
            reports,
            statuses,
            done,
            stop,
            marker,
            _temporary: temporary,
        };
        host.drive = host.await_mounted()?;
        Ok(host)
    }

    /// The host reports `Mounted`; the `mounted` callback confirms the letter.
    fn await_mounted(&self) -> io::Result<DriveLetter> {
        let deadline = Instant::now() + Duration::from_secs(60);
        let drive = loop {
            match self
                .reports
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok((MountStatus::Mounted { drive }, _)) => break drive,
                Ok((MountStatus::Failed { detail }, _))
                | Ok((MountStatus::RuntimeUnavailable { detail }, _)) => {
                    return Err(io::Error::other(format!("host did not mount: {detail}")))
                }
                Ok(_) => {}
                Err(error) => return Err(io::Error::other(format!("no Mounted report: {error}"))),
            }
        };
        loop {
            match self
                .statuses
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(MountStatus::Mounted { drive: mounted }) if mounted == drive => {
                    return Ok(drive)
                }
                Ok(_) => {}
                Err(error) => {
                    return Err(io::Error::other(format!("no mounted callback: {error}")))
                }
            }
        }
    }

    fn root(&self) -> PathBuf {
        PathBuf::from(format!("{}:\\", self.drive.get()))
    }

    /// Ends the host through an update stop or an unmount request; returns its
    /// result and every report after `Mounted`.
    fn finish(self, update_stop: bool) -> io::Result<(Result<HostExit, String>, Vec<Report>)> {
        if update_stop {
            *self
                .marker
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(SystemTime::now());
        } else {
            self.stop.store(true, Ordering::Release);
        }
        let result = self
            .done
            .recv_timeout(Duration::from_secs(120))
            .map_err(|error| io::Error::other(format!("the host did not end: {error}")))?;
        Ok((result, self.reports.try_iter().collect()))
    }
}

impl Drop for MountedHost {
    fn drop(&mut self) {
        // A failed assertion must not leave a mounted drive behind.
        self.stop.store(true, Ordering::Release);
        let _ = self.done.recv_timeout(Duration::from_secs(120));
    }
}

/// Read-only remote with one readable file and two whose reads fail.
struct FailingBackend;

impl FailingBackend {
    fn new() -> Arc<Self> {
        Arc::new(Self)
    }

    fn file(name: &str) -> VfsMeta {
        VfsMeta {
            name: name.into(),
            size: 4,
            mtime_ms: 1_700_000_000_000,
            ..VfsMeta::default()
        }
    }
}

const FILES: [&str; 3] = ["fine.txt", "other.txt", "unreachable.txt"];

impl Backend for FailingBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn case_sensitive_paths(&self, _root: &str) -> bool {
        true
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        if path != "/" {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such directory"));
        }
        Ok(FILES.iter().map(|name| Self::file(name)).collect())
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        if path == "/" {
            return Ok(VfsMeta {
                is_dir: true,
                ..VfsMeta::default()
            });
        }
        FILES
            .iter()
            .find(|name| path.strip_prefix('/') == Some(**name))
            .map(|name| Self::file(name))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        match path {
            "/fine.txt" => Ok(Box::new(io::Cursor::new(b"fine".to_vec()))),
            "/other.txt" => Err(io::Error::other("backend failure without a kind")),
            "/unreachable.txt" => Err(io::Error::new(
                io::ErrorKind::HostUnreachable,
                "the peer is offline",
            )),
            _ => Err(io::Error::new(io::ErrorKind::NotFound, "no such file")),
        }
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(read_only())
    }

    fn rename(&self, _source: &str, _destination: &str) -> VfsResult<()> {
        Err(read_only())
    }

    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Err(read_only())
    }

    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Err(read_only())
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(read_only())
    }
}

fn read_only() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "read-only fixture")
}
