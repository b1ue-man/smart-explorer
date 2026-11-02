//! Linux end-to-end signal of vault drives (VLT-M12, stage linux-root:
//! vault-mount, fixtures `fuse` and `runtime-tmpfs`, one test thread, switch
//! `SMART_EXPLORER_FUSE_TASK=1`): the production host flow over FUSE serves
//! a real vault below `$XDG_RUNTIME_DIR/smart-explorer/Tresore` with the
//! sealed storage of its spool key. A file written through the drive is in
//! the vault after the host ends, a new host with the same key serves it
//! complete, and no cleartext marker lies in the drive cache or the vault
//! storage.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use std::{fs, io};

use super::fuse_host::FuseHost;
use super::mount_point::vaults_root;
use crate::daemon::{MountHostConfig, UpdateStopBaseline, UPDATE_STOP_POLL};
use crate::mount::os::shared::host_session::{
    self, HostControl, HostExit, HostInputs, SpoolKeyAnswer, UpdateStopCheck,
};
use crate::mount::{
    DriveSelection, MountConfig, MountId, MountMode, MountRecovery, MountSource, MountStatus,
    PathDialect,
};
use crate::vault::{create_vault, unlock_storage, SpoolKey, VaultBackend, VaultSession};
use crate::vfs::{Backend, BackendHandle, LocalBackend};

const MARKER: &[u8] = b"KLARTEXT-MARKER-VAULT-FUSE-E2E";
const PASSWORD: &str = "Laufwerk-Passwort-1";
const KEY: [u8; 32] = [0x42; 32];
const LIMIT: Duration = Duration::from_secs(120);

type Report = (MountStatus, Option<MountRecovery>);

struct Control {
    reports: mpsc::Sender<Report>,
    stop: Arc<AtomicBool>,
}

impl HostControl for Control {
    fn report(&self, status: MountStatus, recovery: Option<MountRecovery>) -> Result<(), String> {
        let _ = self.reports.send((status, recovery));
        Ok(())
    }

    fn wait_for_stop(&self, timeout: Duration) -> bool {
        std::thread::sleep(timeout);
        self.stop.load(Ordering::Acquire)
    }

    fn spool_key(&self) -> SpoolKeyAnswer {
        SpoolKeyAnswer::Key(SpoolKey::from_bytes(&KEY).expect("test key"))
    }
}

/// One production FUSE host over the vault view; ends on drop.
struct VaultHost {
    stop: Arc<AtomicBool>,
    done: mpsc::Receiver<Result<HostExit, String>>,
    ended: bool,
}

impl VaultHost {
    fn start(view: BackendHandle, id: &MountId, cache: &Path, folder: &Path) -> io::Result<Self> {
        let config = MountConfig::new(
            id.clone(),
            MountSource::Vault {
                site_id: "0123456789abcdef0123456789abcdef".into(),
            },
            DriveSelection::Automatic,
            MountMode::ReadWrite,
            "Tresor E2E",
        )?
        .with_location_directory(Some(folder.to_path_buf()))?;
        let mut host_config = MountHostConfig::from(&config);
        host_config.path_dialect = PathDialect::Posix;
        host_config.location = config.location.clone();
        let inputs = HostInputs {
            config: host_config,
            backend: view,
            cache_root: cache.to_path_buf(),
        };
        let (report_send, reports) = mpsc::channel();
        let (done_send, done) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("vault-fuse-e2e".into())
            .spawn(move || {
                let baseline =
                    UpdateStopBaseline::with_source(Arc::new(|| None::<std::time::SystemTime>));
                let check = UpdateStopCheck::new(baseline, UPDATE_STOP_POLL);
                let control = Control {
                    reports: report_send,
                    stop: thread_stop,
                };
                let mut platform = FuseHost::default();
                let _ = done_send.send(host_session::run(inputs, &control, &mut platform, &check));
            })?;
        let host = Self {
            stop,
            done,
            ended: false,
        };
        let deadline = Instant::now() + LIMIT;
        loop {
            match reports.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok((MountStatus::MountedAt { path }, _)) if path == folder => return Ok(host),
                Ok((MountStatus::Failed { detail }, _))
                | Ok((MountStatus::RuntimeUnavailable { detail }, _)) => {
                    return Err(io::Error::other(format!("host did not mount: {detail}")))
                }
                Ok(_) => {}
                Err(error) => return Err(io::Error::other(format!("no MountedAt: {error}"))),
            }
        }
    }

    fn finish(mut self) -> Result<HostExit, String> {
        self.stop.store(true, Ordering::Release);
        self.ended = true;
        self.done
            .recv_timeout(LIMIT)
            .map_err(|error| format!("the host did not end: {error}"))?
    }
}

impl Drop for VaultHost {
    fn drop(&mut self) {
        if !self.ended {
            self.stop.store(true, Ordering::Release);
            let _ = self.done.recv_timeout(LIMIT);
        }
    }
}

/// Whether any file below `root` holds `needle`.
fn holds(root: &Path, needle: &[u8]) -> bool {
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => holds(&path, needle),
            Ok(kind) if kind.is_file() => fs::read(&path)
                .is_ok_and(|bytes| bytes.windows(needle.len()).any(|window| window == needle)),
            _ => false,
        }
    })
}

#[test]
#[ignore = "vault task stage linux-root: vault-mount (fuse, runtime-tmpfs, SMART_EXPLORER_FUSE_TASK=1)"]
fn vault_task_mount_linux_fuse_e2e() -> io::Result<()> {
    assert_eq!(
        std::env::var("SMART_EXPLORER_FUSE_TASK").as_deref(),
        Ok("1"),
        "runs only in the stage linux-root: vault-mount (fusermount3, /dev/fuse, tmpfs runtime folder)"
    );
    let base = tempfile::tempdir()?;
    let storage_root = format!("{}/vault", base.path().to_string_lossy());
    let inner: BackendHandle = Arc::new(LocalBackend::new("/"));
    create_vault(inner.as_ref(), &storage_root, PASSWORD, 220, None)
        .map_err(|error| io::Error::other(error.to_string()))?;
    let (key, config) = unlock_storage(inner.as_ref(), &storage_root, PASSWORD, None)
        .map_err(|error| io::Error::other(error.to_string()))?;
    let session = VaultSession::new("e2e-site", key, config);
    let view: BackendHandle = Arc::new(VaultBackend::new(
        Arc::clone(&inner),
        &storage_root,
        Arc::clone(&session),
    ));
    let id = MountId::new_random()?;
    let cache = base.path().join("mount-cache");
    let folder: PathBuf = vaults_root()?.join(format!("Tresor E2E {}", std::process::id()));

    let host = VaultHost::start(Arc::clone(&view), &id, &cache, &folder)?;
    fs::write(folder.join("Bericht.txt"), MARKER)?;
    assert_eq!(host.finish(), Ok(HostExit::Stopped));

    // A new host with the same spool key serves the file complete.
    let host = VaultHost::start(Arc::clone(&view), &id, &cache, &folder)?;
    assert_eq!(fs::read(folder.join("Bericht.txt"))?, MARKER);
    assert_eq!(host.finish(), Ok(HostExit::Stopped));

    // The vault holds the file encrypted; the drive cache holds no
    // cleartext either.
    let mut read = Vec::new();
    io::Read::read_to_end(&mut view.open_read("/Bericht.txt")?, &mut read)?;
    assert_eq!(read, MARKER);
    assert!(!holds(&cache, MARKER), "cleartext in the drive cache");
    assert!(
        !holds(Path::new(&storage_root), MARKER),
        "cleartext in the vault storage"
    );
    let _ = fs::remove_dir(&folder);
    Ok(())
}
