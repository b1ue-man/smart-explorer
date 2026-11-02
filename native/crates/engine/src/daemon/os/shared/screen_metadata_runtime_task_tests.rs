//! A vault drive host at the point of v0.5.186's failure („Spool-Schlüssel
//! lesen: … (os error 10054)“): the real worker listener (capability
//! prelude `ipc_admission.rs`, `ipc.rs::handle_client`, drive table) and the
//! real host session (`MountHostAttach`, private backend stream,
//! `Vault::SpoolKey`, `MountHostStatus`, `MountHostExt`) talk over loopback,
//! and the shared host flow (`host_session::run`: sealed storage from the
//! key, local recovery report, remote root check, preload, reconciled
//! report, volume, running status, teardown) reads a file of the drive.
//!
//! Stand-ins, and where the real parts are covered: the worker's vault
//! register (`session_fixture::inject_spool_key`; `vault_task_spool_key_*`)
//! and the filesystem frontend (Dokany/FUSE platform; `drive_task_*`).
use std::io;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use super::{attach, HostEnvironment, MountHostSession};
use crate::daemon::drive_modules::ipc_protocol_mount_host::MountHostRequest;
use crate::daemon::ipc_host::ShareHost;
use crate::daemon::ipc_listener::serve_for_test;
use crate::daemon::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse, VaultIpcRequest,
};
use crate::daemon::mount_manager::session_fixture::inject_spool_key;
use crate::daemon::{MountHostConfig, UpdateStopBaseline};
use crate::mount::optimization_fixture::{FixtureDirectory, OptimizationBackend};
use crate::mount::os::shared::host_platform::{
    HostLease, HostPlatform, HostStepError, VolumeLocation, VolumePoll,
};
use crate::mount::os::shared::host_session::{
    run, HostControl, HostExit, HostInputs, SpoolKeyAnswer, UpdateStopCheck,
};
use crate::mount::{
    CacheSpaceProbe, DriveSelection, MountCapacityReport, MountConfig, MountEngine, MountId,
    MountMode, MountRecovery, MountSource, MountStatus, OpenDisposition, OpenFileOptions,
    PathDialect,
};
use crate::vfs::{Backend, BackendHandle, Scheme, VfsMeta, VfsResult};

const DAEMON: &str = "screen-metadata-runtime-daemon-token";
const KEY: [u8; 32] = [0x6b; 32];
const CONTENT: &[u8] = b"KLARTEXT-MARKER Private Unterlagen";
const SITE: &str = "0123456789abcdef0123456789abcdef";
const NOT_THE_TREE: &str = "Nur der Gerätebaum darf Geräte abfragen";

/// The cleartext view of a vault as the worker opens it (`Scheme::Vault`).
struct VaultView(Arc<OptimizationBackend>);

impl Backend for VaultView {
    fn scheme(&self) -> Scheme {
        Scheme::Vault
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.0.list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.0.stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn io::Read + Send>> {
        self.0.open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn io::Write + Send>> {
        self.0.open_write(path)
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.0.rename(src, dst)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.0.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.0.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.0.mkdir_all(path)
    }
}

/// The host's daemon side: every request goes over the real session; the
/// first control poll ends the drive like an unmount request.
struct Control<'a>(&'a MountHostSession);

impl HostControl for Control<'_> {
    fn report_capacity(&self, capacity: MountCapacityReport) -> Result<(), String> {
        self.0.report_capacity(capacity)
    }
    fn report(&self, status: MountStatus, recovery: Option<MountRecovery>) -> Result<(), String> {
        self.0.report_status_with_recovery(status, recovery)
    }
    fn wait_for_stop(&self, _timeout: Duration) -> bool {
        true
    }
    fn spool_key(&self) -> SpoolKeyAnswer {
        self.0.request_spool_key()
    }
}

struct Unlimited;

impl CacheSpaceProbe for Unlimited {
    fn available_bytes(&self) -> io::Result<u64> {
        Ok(u64::MAX / 4)
    }
}

/// A frontend whose program opens `note.txt` as soon as the volume runs.
struct Volume {
    dialect: PathDialect,
    place: PathBuf,
    engine: Option<Arc<MountEngine>>,
    read: Arc<Mutex<Option<Vec<u8>>>>,
}

impl HostPlatform for Volume {
    fn path_dialect(&self) -> PathDialect {
        self.dialect
    }
    fn acquire_lease(&mut self, _spool_root: &Path, _id: &MountId) -> io::Result<HostLease> {
        Ok(Box::new(()))
    }
    fn space_probe(&mut self, _spool_root: &Path) -> io::Result<Arc<dyn CacheSpaceProbe>> {
        Ok(Arc::new(Unlimited))
    }
    fn select_runtime(
        &mut self,
        _spool_root: &Path,
        _config: &MountHostConfig,
    ) -> Result<(), String> {
        Ok(())
    }
    fn prepare_volume(
        &mut self,
        engine: &Arc<MountEngine>,
        _config: &MountHostConfig,
        _spool_root: &Path,
    ) -> Result<(), HostStepError> {
        self.engine = Some(Arc::clone(engine));
        Ok(())
    }
    fn start_volume(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn start_background(&mut self) -> io::Result<()> {
        let engine = self
            .engine
            .as_ref()
            .ok_or_else(|| io::Error::other("no prepared volume"))?;
        let path = if self.dialect == PathDialect::Posix {
            "/note.txt"
        } else {
            "\\note.txt"
        };
        let options = OpenFileOptions {
            writable: false,
            disposition: OpenDisposition::OpenExisting,
        };
        let handle = engine.open_file(path, options)?;
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 64];
        loop {
            let read = engine.read(handle, bytes.len() as u64, &mut chunk)?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        engine.close(handle)?;
        *self.read.lock().unwrap() = Some(bytes);
        Ok(())
    }
    fn location(&self) -> io::Result<VolumeLocation> {
        Ok(VolumeLocation::Directory(self.place.clone()))
    }
    fn stop_requested(&self) -> bool {
        false
    }
    fn poll_volume(&mut self) -> VolumePoll {
        VolumePoll::Running
    }
    fn deliver_changes(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn shutdown(&mut self) -> bool {
        true
    }
    fn release_volume(&mut self) {
        self.engine = None;
    }
    fn finish_runtime(&mut self, _completed: bool) {}
}

/// One request on its own connection, answered on that connection.
fn ask(addr: SocketAddr, request: &IpcRequest) -> IpcResponse {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    set_stream_timeout(&stream, Some(Duration::from_secs(10)));
    write_request(&mut stream, request).unwrap();
    read_response(&mut stream)
        .unwrap_or_else(|error| panic!("no explicit answer, the connection ended: {error}"))
}

fn spool_key(token: &str, mount_id: &str) -> IpcRequest {
    IpcRequest::Vault {
        token: token.into(),
        req: VaultIpcRequest::SpoolKey {
            mount_id: mount_id.into(),
        },
    }
}

/// The text of an explicit refusal; it never names `secret`.
fn refused(response: IpcResponse, secret: &str) -> String {
    match response {
        IpcResponse::Err { msg, code } => {
            assert_eq!(
                code.as_deref(),
                Some(crate::daemon::ipc::AUTH_REJECTED_CODE),
                "{msg}"
            );
            assert!(!msg.contains(secret), "the refusal names a token");
            msg
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn tree_contains(root: &Path, needle: &[u8]) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        if path.is_dir() {
            tree_contains(&path, needle)
        } else {
            let bytes = std::fs::read(&path).unwrap_or_default();
            bytes.windows(needle.len()).any(|window| window == needle)
        }
    })
}

#[test]
fn screen_metadata_runtime_task_vault_host_gets_spool_key_and_reads_file() {
    let directory = FixtureDirectory::new();
    let remote = OptimizationBackend::new();
    remote.put("/note.txt", CONTENT);
    let backend: BackendHandle = Arc::new(VaultView(Arc::clone(&remote)));
    let host = ShareHost::new("7".repeat(32));
    let id = MountId::new_random().unwrap();
    let config = MountConfig::new(
        id.clone(),
        MountSource::Vault {
            site_id: SITE.into(),
        },
        DriveSelection::Automatic,
        MountMode::ReadOnly,
        "Private Unterlagen",
    )
    .unwrap();
    let launch_token = "a1".repeat(32);
    host.mounts
        .arm_host_for_test(config, backend, &launch_token)
        .unwrap();
    inject_spool_key(&id, KEY);
    let server = serve_for_test(host.clone(), DAEMON.into()).unwrap();
    let addr = server.addr();
    let cache_root = crate::mount::prepare_spool_root(&directory.path().join("cache")).unwrap();

    // Attach as the host process does (launch capability → session).
    let session = attach(
        id.clone(),
        HostEnvironment {
            launch_token,
            ipc_addr: addr,
            cache_root,
        },
    )
    .unwrap();
    assert!(session.vault, "the grant names the vault scheme");
    let session_token = session.session_token.clone();

    // Refusals at this point are explicit answers, never a reset.
    let unknown = "f0".repeat(32);
    refused(ask(addr, &spool_key(&unknown, id.as_str())), &unknown);
    let other_mount = MountId::new_random().unwrap();
    let text = refused(
        ask(addr, &spool_key(&session_token, other_mount.as_str())),
        &session_token,
    );
    assert_eq!(text, "Unbekannter Laufwerk-Host");
    let text = refused(ask(addr, &spool_key(DAEMON, id.as_str())), DAEMON);
    assert_eq!(text, "Laufwerk-Session-Token wurde abgelehnt");
    let foreign_ext = IpcRequest::MountHostExt {
        id: id.clone(),
        session_token: unknown.clone(),
        req: MountHostRequest::DeviceWatch { since: 0 },
    };
    refused(ask(addr, &foreign_ext), &unknown);
    // The drive's own extension channel passes the prelude and the session
    // check; only the device tree may watch devices.
    let own_ext = IpcRequest::MountHostExt {
        id: id.clone(),
        session_token: session_token.clone(),
        req: MountHostRequest::DeviceWatch { since: 0 },
    };
    match ask(addr, &own_ext) {
        IpcResponse::Err { msg, code: None } => assert_eq!(msg, NOT_THE_TREE),
        other => panic!("MountHostExt of the drive was not served: {other:?}"),
    }

    // The host's own request, exactly where the shared flow makes it.
    match session.request_spool_key() {
        SpoolKeyAnswer::Key(key) => assert_eq!(key.expose(), &KEY),
        SpoolKeyAnswer::Locked(text) | SpoolKeyAnswer::Failed(text) => {
            panic!("the host got no spool key: {text}")
        }
    }

    // The whole start: key → sealed storage → recovery reports → running
    // volume, on which a program reads the file; then a clean end.
    let read = Arc::new(Mutex::new(None));
    let mut volume = Volume {
        dialect: session.config.path_dialect,
        place: directory.path().join("Private Unterlagen"),
        engine: None,
        read: Arc::clone(&read),
    };
    let check = UpdateStopCheck::new(
        UpdateStopBaseline::with_source(Arc::new(|| -> Option<SystemTime> { None })),
        Duration::from_secs(3600),
    );
    let result = run(
        HostInputs::from_session(&session),
        &Control(&session),
        &mut volume,
        &check,
    );
    assert_eq!(result, Ok(HostExit::Stopped));
    assert_eq!(read.lock().unwrap().as_deref(), Some(CONTENT));
    let row = host.mounts.snapshot_for_test(&id).expect("the drive row");
    assert_eq!(row.status, MountStatus::Unmounted);
    assert_eq!(row.recovery, MountRecovery::Clean);
    assert!(
        !tree_contains(&session.cache_root, CONTENT),
        "cleartext reached the drive cache"
    );

    drop(session);
    host.mounts.forget_for_test(&id);
    drop(server);
}
