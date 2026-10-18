//! Windows end-to-end signals of vault drives (DRV-M7 stage windows-core:
//! drive-vault, VLT-M12 stage windows-core: vault-mount; fixture `dokany`,
//! one test thread, `--include-ignored`): the production Dokany host serves
//! a real vault as a drive letter with the sealed storage of its spool key.
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use super::super::super::shared::host_session::{
    self, HostControl, HostExit, HostInputs, SpoolKeyAnswer, UpdateStopCheck,
};
use super::super::callback_reporter::CallbackReporter;
use super::DokanyHost;
use crate::daemon::{MountHostConfig, UpdateStopBaseline, UPDATE_STOP_POLL};
use crate::mount::{
    DriveLetter, DriveSelection, MountConfig, MountId, MountMode, MountRecovery, MountSource,
    MountStatus,
};
use crate::vault::{create_vault, unlock_storage, SpoolKey, VaultBackend, VaultSession};
use crate::vfs::{Backend, BackendHandle, LocalBackend};

const MARKER: &[u8] = b"KLARTEXT-MARKER-VAULT-DOKANY-E2E";
const PASSWORD: &str = "Laufwerk-Passwort-1";
const KEY: [u8; 32] = [0x24; 32];
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

/// A real vault in a temporary folder and its cleartext view.
struct Vault {
    base: tempfile::TempDir,
    storage_root: String,
    view: BackendHandle,
}

fn vault() -> io::Result<Vault> {
    let base = tempfile::tempdir()?;
    let storage_root = base
        .path()
        .join("vault")
        .to_string_lossy()
        .replace('\\', "/");
    let inner: BackendHandle = Arc::new(LocalBackend::new("/"));
    create_vault(inner.as_ref(), &storage_root, PASSWORD, 220, None)
        .map_err(|error| io::Error::other(error.to_string()))?;
    let (key, config) = unlock_storage(inner.as_ref(), &storage_root, PASSWORD, None)
        .map_err(|error| io::Error::other(error.to_string()))?;
    let session = VaultSession::new("e2e-site", key, config);
    let view: BackendHandle = Arc::new(VaultBackend::new(inner, &storage_root, session));
    Ok(Vault {
        base,
        storage_root,
        view,
    })
}

/// One production Dokany host over the vault view; ends on drop.
struct VaultHost {
    drive: DriveLetter,
    stop: Arc<AtomicBool>,
    done: mpsc::Receiver<Result<HostExit, String>>,
    _statuses: mpsc::Receiver<MountStatus>,
    ended: bool,
}

impl VaultHost {
    fn start(view: BackendHandle, id: &MountId, cache: &Path) -> io::Result<Self> {
        let config = MountConfig::new(
            id.clone(),
            MountSource::Vault {
                site_id: "0123456789abcdef0123456789abcdef".into(),
            },
            DriveSelection::Automatic,
            MountMode::ReadWrite,
            "Tresor E2E",
        )?;
        let inputs = HostInputs {
            config: MountHostConfig::from(&config),
            backend: view,
            cache_root: cache.to_path_buf(),
        };
        let (report_send, reports) = mpsc::channel();
        let (status_send, statuses) = mpsc::channel();
        let (done_send, done) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("vault-dokany-e2e".into())
            .spawn(move || {
                let baseline =
                    UpdateStopBaseline::with_source(Arc::new(|| None::<std::time::SystemTime>));
                let check = UpdateStopCheck::new(baseline, UPDATE_STOP_POLL);
                let control = Control {
                    reports: report_send,
                    stop: thread_stop,
                };
                let mut platform = DokanyHost::new(CallbackReporter::Capture(status_send));
                let _ = done_send.send(host_session::run(inputs, &control, &mut platform, &check));
            })?;
        let deadline = Instant::now() + LIMIT;
        let drive = loop {
            match reports.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok((MountStatus::Mounted { drive }, _)) => break drive,
                Ok((MountStatus::Failed { detail }, _))
                | Ok((MountStatus::RuntimeUnavailable { detail }, _)) => {
                    return Err(io::Error::other(format!("host did not mount: {detail}")))
                }
                Ok(_) => {}
                Err(error) => return Err(io::Error::other(format!("no Mounted: {error}"))),
            }
        };
        // The volume answers once the `mounted` callback confirmed it.
        loop {
            match statuses.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(MountStatus::Mounted { drive: mounted }) if mounted == drive => break,
                Ok(_) => {}
                Err(error) => {
                    return Err(io::Error::other(format!("no mounted callback: {error}")))
                }
            }
        }
        Ok(Self {
            drive,
            stop,
            done,
            _statuses: statuses,
            ended: false,
        })
    }

    fn path(&self, name: &str) -> PathBuf {
        PathBuf::from(format!("{}:\\{name}", self.drive.get()))
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

fn vault_bytes(view: &BackendHandle, path: &str) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    view.open_read(path)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[test]
#[ignore = "requires the pinned Dokany runtime and its installed driver (windows-core: drive-vault)"]
fn drive_task_vault_drive_letter_rw() -> io::Result<()> {
    let vault = vault()?;
    let cache = vault.base.path().join("mount-cache");
    let id = MountId::new_random()?;
    let host = VaultHost::start(Arc::clone(&vault.view), &id, &cache)?;
    fs::write(host.path("Bericht.txt"), MARKER)?;
    assert_eq!(fs::read(host.path("Bericht.txt"))?, MARKER);
    fs::create_dir(host.path("Ordner"))?;
    fs::write(host.path("Ordner\\Notiz.txt"), b"Notiz")?;
    assert_eq!(host.finish(), Ok(HostExit::Stopped));
    assert_eq!(vault_bytes(&vault.view, "/Bericht.txt")?, MARKER);
    assert_eq!(vault_bytes(&vault.view, "/Ordner/Notiz.txt")?, b"Notiz");
    assert!(!holds(&cache, MARKER), "cleartext in the drive cache");
    assert!(
        !holds(Path::new(&vault.storage_root), MARKER),
        "cleartext in the vault storage"
    );
    Ok(())
}

#[test]
#[ignore = "requires the pinned Dokany runtime and its installed driver (windows-core: vault-mount)"]
fn vault_task_mount_windows_dokany_e2e() -> io::Result<()> {
    let vault = vault()?;
    let cache = vault.base.path().join("mount-cache");
    let id = MountId::new_random()?;
    let host = VaultHost::start(Arc::clone(&vault.view), &id, &cache)?;
    fs::write(host.path("Bericht.txt"), MARKER)?;
    assert_eq!(host.finish(), Ok(HostExit::Stopped));
    // A new host with the same spool key serves the file complete.
    let host = VaultHost::start(Arc::clone(&vault.view), &id, &cache)?;
    assert_eq!(fs::read(host.path("Bericht.txt"))?, MARKER);
    assert_eq!(host.finish(), Ok(HostExit::Stopped));
    assert!(!holds(&cache, MARKER), "cleartext in the drive cache");
    Ok(())
}
