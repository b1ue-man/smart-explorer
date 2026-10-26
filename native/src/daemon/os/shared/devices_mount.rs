//! Starting and stopping the device tree „Verbundene Geräte als Ordner“ in
//! the worker (DRV-M6, U17-23 and U17-24), and its default „ein“.
//!
//! The tree is one drive, `se-devices`, with its own source
//! (`MountSource::Devices`). It needs no backend of its own: the host
//! reaches every device through further connections (`devices_host.rs`), so
//! the handshake of every drive hands it a placeholder backend that lists an
//! empty root. The registry keeps the entry in `registry-2.json`; the
//! drive restores at login until the user switches it off.
//!
//! Switching off always works: the host unmounts, the registry entry goes
//! even when unsaved changes of devices remain, and the caches of the
//! children stay for the next switch-on (their journals are never deleted).
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::mount::{
    devices_label, DriveLetter, DriveSelection, MountConfig, MountId, MountMode, MountSnapshot,
    MountSource, DEVICES_MOUNT_ID,
};
use crate::vfs::{Backend, BackendHandle, Scheme, StagedWriteCapabilities, VfsMeta, VfsResult};

use super::super::ipc_host::ShareHost;
use super::devices_host::{observe_status, projection};
use super::devices_ipc::{build_status, host_dialect, DevicesStatus};

/// The decision of the user about the device tree, kept beside the registry.
const CHOICE_FILE: &str = "devices.json";
/// Once per service process the default may switch the tree on.
static DEFAULT_TRIED: AtomicBool = AtomicBool::new(false);

/// The backend of the handshake: an empty root and nothing else. The host
/// never calls it.
pub(super) struct DevicesRootBackend;

fn nothing<T>() -> VfsResult<T> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Der Gerätebaum hat kein eigenes Backend",
    ))
}

impl Backend for DevicesRootBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Peer
    }

    fn root_display(&self) -> String {
        "/".into()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        if path == "/" {
            Ok(Vec::new())
        } else {
            nothing()
        }
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        if path == "/" {
            Ok(VfsMeta {
                is_dir: true,
                ..VfsMeta::default()
            })
        } else {
            nothing()
        }
    }

    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        nothing()
    }

    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        nothing()
    }

    fn rename(&self, _source: &str, _destination: &str) -> VfsResult<()> {
        nothing()
    }

    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        nothing()
    }

    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        nothing()
    }

    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        nothing()
    }

    fn staged_write_capabilities(&self, _root: &str) -> StagedWriteCapabilities {
        StagedWriteCapabilities::complete()
    }

    fn parallelism(&self) -> usize {
        1
    }
}

/// The backend the start path hands to the host of the device tree.
pub(crate) fn placeholder_backend() -> BackendHandle {
    std::sync::Arc::new(DevicesRootBackend)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Choice {
    On,
    Off,
}

#[derive(Serialize, Deserialize)]
struct ChoiceFile {
    choice: Choice,
}

fn mounts_directory() -> PathBuf {
    crate::support_dirs::app_data_dir().join("mounts")
}

/// What the user decided; `None` while nothing was decided or the file does
/// not read (then the default may apply).
pub(super) fn read_choice_in(directory: &Path) -> Option<Choice> {
    let id = MountId::parse(DEVICES_MOUNT_ID).ok()?;
    // The registry's one atomic ownership commit is also the effective
    // switch-off decision. An old devices.json:On must never auto-enable
    // an archived tree; a registry read error cannot authorize auto-enable.
    match super::super::mount_registry::inactive::is_inactive_in(directory, &id) {
        Ok(false) => {}
        Ok(true) | Err(_) => return Some(Choice::Off),
    }
    let body = std::fs::read(directory.join(CHOICE_FILE)).ok()?;
    serde_json::from_slice::<ChoiceFile>(&body)
        .ok()
        .map(|file| file.choice)
}

pub(super) fn write_choice_in(directory: &Path, choice: Choice) -> io::Result<()> {
    let body = serde_json::to_vec(&ChoiceFile { choice }).map_err(io::Error::other)?;
    crate::support_dirs::write_private_atomic(&directory.join(CHOICE_FILE), &body)
}

/// The drive entry of the device tree in the table of the worker.
fn tree_entry(host: &ShareHost) -> Option<MountSnapshot> {
    host.mounts
        .list()
        .ok()?
        .into_iter()
        .find(|mount| mount.config.source.is_device_tree())
}

/// The answer about the tree now.
pub(crate) fn status(host: &ShareHost) -> DevicesStatus {
    let entry = tree_entry(host);
    let spool = crate::support_dirs::app_data_dir().join("mount-cache");
    observe_status(build_status(
        entry.as_ref(),
        &projection(host),
        crate::mount::pending_devices(&spool),
        host_dialect(),
    ))
}

/// `DevicesEnable`: registers and starts the tree and records the decision.
pub(crate) fn enable(
    host: &ShareHost,
    letter: Option<String>,
    mount_point: Option<String>,
) -> Result<DevicesStatus, String> {
    if !crate::mount::drive_mount_supported() {
        return Err("Laufwerke gibt es nur unter Windows und Linux".into());
    }
    if tree_entry(host).is_some() {
        return Err("Der Gerätebaum ist schon eingeschaltet".into());
    }
    let id = MountId::parse(DEVICES_MOUNT_ID).map_err(|error| error.to_string())?;
    let mut config = MountConfig::new(
        id,
        MountSource::Devices,
        drive_of(letter.as_deref(), mount_point.is_some())?,
        MountMode::ReadWrite,
        devices_label(),
    )
    .map_err(|error| error.to_string())?;
    if let Some(folder) = mount_point.as_deref().filter(|_| cfg!(not(windows))) {
        config = config
            .with_location_directory(Some(PathBuf::from(folder)))
            .map_err(|error| format!("Einhängeort: {error}"))?;
    }
    host.mounts.start(config, host)?;
    write_choice_in(&mounts_directory(), Choice::On)
        .map_err(|error| format!("Entscheidung speichern: {error}"))?;
    Ok(status(host))
}

/// The drive letter a switch-on asks for (Windows) and the check that the
/// other platform's option is absent.
fn drive_of(letter: Option<&str>, has_mount_point: bool) -> Result<DriveSelection, String> {
    let letter = letter.map(str::trim).filter(|letter| !letter.is_empty());
    if cfg!(windows) {
        if has_mount_point {
            return Err(
                "Einhängeorte gibt es nur unter Linux; nutze den Laufwerksbuchstaben".into(),
            );
        }
        let Some(letter) = letter.filter(|letter| !letter.eq_ignore_ascii_case("auto")) else {
            return Ok(DriveSelection::Automatic);
        };
        let mut characters = letter.trim_end_matches([':', '\\', '/']).chars();
        return match (characters.next(), characters.next()) {
            (Some(single), None) => DriveLetter::parse(single)
                .map(DriveSelection::Letter)
                .map_err(|error| error.to_string()),
            _ => Err("Der Laufwerksbuchstabe muss `auto` oder ein Buchstabe sein".into()),
        };
    }
    if letter.is_some() {
        return Err("Laufwerksbuchstaben gibt es nur unter Windows; nutze den Einhängeort".into());
    }
    Ok(DriveSelection::Automatic)
}

/// `DevicesDisable`: stops the host, forgets the registry entry in every
/// case and records the decision. The caches of the children stay.
pub(crate) fn disable(host: &ShareHost) -> Result<DevicesStatus, String> {
    let id = MountId::parse(DEVICES_MOUNT_ID).map_err(|error| error.to_string())?;
    let stopped = if tree_entry(host).is_some() {
        host.mounts.stop(&id).map(|_| ())
    } else {
        Ok(())
    };
    host.mounts.forget_idle(&id);
    super::super::mount_registry::remove(&id)
        .and_then(|()| write_choice_in(&mounts_directory(), Choice::Off))
        .map_err(|error| format!("Gerätebaum vergessen: {error}"))?;
    stopped?;
    Ok(status(host))
}

/// The registry entry goes and the decision becomes „aus“, in `directory`
/// (what `disable` does to the files; the tests run it on a temporary one).
#[cfg(test)]
pub(super) fn forget_in(directory: &Path, id: &MountId) -> io::Result<()> {
    super::super::mount_registry::remove_in(directory, id)?;
    write_choice_in(directory, Choice::Off)
}

/// What the default needs to know and do; the worker implements it over
/// the registry and the profiles, the tests over a fake.
pub(super) trait DefaultWorld {
    fn decided(&self) -> bool;
    fn registered(&self) -> bool;
    fn supported(&self) -> bool;
    fn runtime_ready(&self) -> bool;
    fn has_devices(&self) -> bool;
    fn enable(&self) -> Result<(), String>;
    fn report(&self, text: &str);
}

/// The default „ein“ (decision C5, V11): once per service process, when the
/// user decided nothing, no tree is registered, drives are supported, the
/// runtime is ready and at least one device is paired. A failure writes no
/// decision, so the next start tries again.
pub(super) fn apply_default_with(tried: &AtomicBool, world: &dyn DefaultWorld) {
    if tried.swap(true, Ordering::SeqCst) {
        return;
    }
    if world.decided()
        || world.registered()
        || !world.supported()
        || !world.runtime_ready()
        || !world.has_devices()
    {
        return;
    }
    if let Err(error) = world.enable() {
        world.report(&format!("Gerätebaum automatisch einbinden: {error}"));
    }
}

struct Worker<'a>(&'a ShareHost);

impl DefaultWorld for Worker<'_> {
    fn decided(&self) -> bool {
        read_choice_in(&mounts_directory()).is_some()
    }

    fn registered(&self) -> bool {
        tree_entry(self.0).is_some()
    }

    fn supported(&self) -> bool {
        crate::mount::drive_mount_supported()
    }

    fn runtime_ready(&self) -> bool {
        crate::mount::drive_runtime_ready()
    }

    fn has_devices(&self) -> bool {
        !projection(self.0).is_empty()
    }

    fn enable(&self) -> Result<(), String> {
        enable(self.0, None, None).map(|_| ())
    }

    fn report(&self, text: &str) {
        super::super::state::log(text);
    }
}

/// At the end of the drive restore of an unrestricted worker.
pub(crate) fn apply_default(host: &ShareHost) {
    apply_default_with(&DEFAULT_TRIED, &Worker(host));
}
