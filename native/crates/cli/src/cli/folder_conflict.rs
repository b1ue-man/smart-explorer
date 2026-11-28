//! Terminal folder policy. File overwrite/force remains a separate choice.
use super::target::Target;
use crate::transfer::FolderConflict;
use crate::transfer::StoredEndpoint;
use crate::transfer::{
    read_record, record_context, record_locations, write_record, RecordContext, RecordKind,
};
use crate::vfs::VfsMeta;
use serde::{Deserialize, Serialize};

#[path = "transfer_recovery.rs"]
mod recovery;
#[cfg(test)]
#[path = "folder_conflict_tests.rs"]
mod tests;
pub(crate) use recovery::{RecoveredCommandTask, SavedCommandsCatalog};

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub(super) enum FolderPolicy {
    #[default]
    Merge,
    KeepBoth,
    Fail,
}

impl From<FolderPolicy> for FolderConflict {
    fn from(policy: FolderPolicy) -> Self {
        match policy {
            FolderPolicy::Merge => Self::Merge,
            FolderPolicy::KeepBoth => Self::KeepBoth,
            FolderPolicy::Fail => Self::Fail,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct SourceStamp {
    name: String,
    id: Option<String>,
    directory: bool,
    link: bool,
    size: u64,
    time: i64,
    md5: Option<String>,
    object: Option<String>,
    /// `location_identity` of a source folder without `object` (FAT, exFAT,
    /// FUSE); absent in records written before 2026-11-27.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    location: Option<String>,
}

impl SourceStamp {
    fn capture(source: &Target, meta: &VfsMeta) -> Result<Self, String> {
        let folder = meta.is_dir && !meta.is_symlink;
        let object = if folder {
            crate::vfs::directory_identity(&*source.backend, &source.path)
                .ok()
                .flatten()
        } else {
            None
        };
        let location = if folder && object.is_none() {
            location_identity(&*source.backend, &source.path)
        } else {
            None
        };
        Ok(Self {
            name: meta.name.clone(),
            id: meta.id.clone(),
            directory: meta.is_dir,
            link: meta.is_symlink,
            size: meta.size,
            time: meta.mtime_ms,
            md5: meta.content_md5.clone(),
            object,
            location,
        })
    }
    fn validate(&self, source: &Target, meta: &VfsMeta) -> Result<(), String> {
        let before = VfsMeta {
            name: self.name.clone(),
            id: self.id.clone(),
            is_dir: self.directory,
            is_symlink: self.link,
            size: self.size,
            mtime_ms: self.time,
            content_md5: self.md5.clone(),
            ..Default::default()
        };
        super::tree_guard::validate_same_source(&before, meta, &source.path)?;
        if self.directory && !self.link {
            let actual = crate::vfs::directory_identity(&*source.backend, &source.path)
                .map_err(|error| error.to_string())?;
            let confirmed = match (&self.object, &self.location) {
                (Some(object), _) => actual.as_deref() == Some(object.as_str()),
                (None, Some(location)) => {
                    self.id.is_some()
                        || location_identity(&*source.backend, &source.path).as_deref()
                            == Some(location.as_str())
                }
                (None, None) => self.id.is_some(),
            };
            if !confirmed {
                return Err("Gespeicherter Quellordner ist geändert oder unbestätigt".into());
            }
        }
        Ok(())
    }
}

/// The weaker, named identity of a local folder on a filesystem without
/// stable object ids (FAT, exFAT, FUSE; U4):
/// `volume-location:v1:<volume id>:<creation ms or ->:<path in the volume>`.
/// It confirms the same volume (filesystem UUID or volume serial) and the
/// same place on it, independent of mount point and drive letter, and where
/// the system reports a folder's creation time (Windows) also the same
/// folder there. A folder created anew at that place without a creation
/// time passes, so a resumed run still compares every existing destination
/// file by content (`tree_apply`). `None` off local disks and where the
/// volume has no identity.
pub(super) fn location_identity(backend: &dyn crate::vfs::Backend, path: &str) -> Option<String> {
    if !backend.is_local() {
        return None;
    }
    let volume = crate::vfs::volume_identity(backend, path).ok().flatten()?;
    let meta = backend.stat(path).ok()?;
    if !meta.is_dir || meta.is_symlink {
        return None;
    }
    let created = if meta.btime_ms != 0 {
        meta.btime_ms.to_string()
    } else {
        "-".to_string()
    };
    Some(format!(
        "volume-location:v1:{}:{created}:{}",
        volume.volume_id, volume.relative_path
    ))
}

/// The destination's parent is still the folder the job recorded: its
/// stable identity, or for a record of a filesystem without one its
/// `location_identity`.
fn confirm_parent(record: &SavedCommand, target: &Target) -> Result<(), String> {
    let parent_object = crate::vfs::directory_identity(&*target.backend, &record.parent)
        .map_err(|error| error.to_string())?;
    let confirmed = match (&record.parent_object, &record.parent_location) {
        (Some(object), _) => parent_object.as_deref() == Some(object.as_str()),
        (None, Some(location)) => {
            location_identity(&*target.backend, &record.parent).as_deref()
                == Some(location.as_str())
        }
        (None, None) => false,
    };
    if confirmed {
        Ok(())
    } else {
        Err("Gespeicherter Zielroot ist geändert oder unbestätigt".into())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SavedCommand {
    pub version: u32,
    pub id: String,
    pub source: StoredEndpoint,
    pub target: StoredEndpoint,
    pub source_path: String,
    pub vault_locations: Vec<String>,
    source_local_root: Option<String>,
    pub target_argument: String,
    target_local_root: Option<String>,
    pub destination: String,
    parent: String,
    parent_object: Option<String>,
    /// `location_identity` of `parent` where it has no `parent_object`;
    /// absent in records written before 2026-11-27.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_location: Option<String>,
    pub recursive: bool,
    pub force: bool,
    pub moving: bool,
    pub folders: FolderConflict,
    stamp: SourceStamp,
    pub complete: bool,
    mutation_started: bool,
}

fn endpoint(target: &Target) -> StoredEndpoint {
    StoredEndpoint {
        local: target.backend.is_local(),
        namespace: target.backend.namespace_identity(),
        state: target.backend.state_identity(),
    }
}

fn record_path(id: &str) -> Result<std::path::PathBuf, String> {
    if id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Ungültige Transfer-Jobkennung".into());
    }
    Ok(crate::support_dirs::app_data_file("transfer-cli-jobs").join(format!("{id}.json")))
}

pub(super) struct CommandContext {
    path: std::path::PathBuf,
    _lock: std::fs::File,
    context: Option<RecordContext>,
    pub record: SavedCommand,
}

impl CommandContext {
    pub fn finish_confirmed_move(
        source: &Target,
        target: &Target,
        recursive: bool,
        force: bool,
        folders: Option<FolderConflict>,
        id: &str,
    ) -> Result<Option<String>, String> {
        match source.backend.stat(&source.path) {
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        let locations = record_locations(
            crate::transfer::Side::Remote(&*source.backend),
            crate::transfer::Side::Remote(&*target.backend),
            &[source.path.as_str()],
            &target.path,
        )
        .map_err(|error| error.to_string())?;
        let context = record_context(&locations).map_err(|error| error.to_string())?;
        let path = record_path(id)?;
        let root = path
            .parent()
            .ok_or_else(|| "Jobablage hat keinen Parent".to_string())?;
        let lock = crate::support_dirs::open_private_lock(&root.join(format!("{id}.lock")))
            .map_err(|error| error.to_string())?;
        lock.try_lock()
            .map_err(|error| format!("Job {id} wird bereits ausgeführt: {error}"))?;
        let mut record: SavedCommand =
            read_record(&path, RecordKind::Job, id, u64::MAX, context.is_some())
                .map_err(|error| error.to_string())?;
        if record.version != 1
            || record.id != id
            || record.vault_locations != locations
            || !record.moving
            || record.complete
            || record.source != endpoint(source)
            || record.target != endpoint(target)
            || record.source_path != source.path
            || record.target_argument != target.path
            || record.recursive != recursive
            || record.force != force
            || folders.is_some_and(|folders| folders != record.folders)
        {
            return Err("Gespeicherter Movevertrag stimmt nicht überein".into());
        }
        confirm_parent(&record, target)?;
        let expected = record
            .stamp
            .object
            .as_deref()
            .ok_or_else(|| "Fehlende Quelle hat keinen dauerhaften Movebeleg".to_string())?;
        let actual = if record.folders == FolderConflict::Merge {
            if crate::vfs::directory_identity(&*target.backend, &record.destination)
                .map_err(|error| error.to_string())?
                .as_deref()
                != Some(expected)
            {
                return Err("Fehlende Quelle ist im gespeicherten Moveziel nicht bestätigt".into());
            }
            record.destination.clone()
        } else {
            crate::transfer::engine::folder_reservation::confirmed_named_move(
                &*source.backend,
                &source.path,
                &*target.backend,
                &record.destination,
                id,
                expected,
            )
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Fehlende Quelle besitzt keinen bestätigten Zielalias".to_string())?
        };
        record.complete = true;
        write_record(&path, RecordKind::Job, id, context.as_ref(), &record)
            .map_err(|error| error.to_string())?;
        Ok(Some(actual))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        source: &Target,
        target: &Target,
        meta: &VfsMeta,
        destination: String,
        recursive: bool,
        force: bool,
        moving: bool,
        folders: Option<FolderConflict>,
        resume: Option<&str>,
    ) -> Result<Self, String> {
        let locations = record_locations(
            crate::transfer::Side::Remote(&*source.backend),
            crate::transfer::Side::Remote(&*target.backend),
            &[source.path.as_str()],
            &target.path,
        )
        .map_err(|error| error.to_string())?;
        let context = record_context(&locations).map_err(|error| error.to_string())?;
        let id = resume
            .map(str::to_string)
            .unwrap_or_else(crate::transfer::new_job_id);
        let path = record_path(&id)?;
        let root = path
            .parent()
            .ok_or_else(|| "Jobablage hat keinen Parent".to_string())?;
        crate::support_dirs::ensure_private_dir(root).map_err(|error| error.to_string())?;
        let lock = crate::support_dirs::open_private_lock(&root.join(format!("{id}.lock")))
            .map_err(|error| error.to_string())?;
        lock.try_lock()
            .map_err(|error| format!("Job {id} wird bereits ausgeführt: {error}"))?;
        let record = if resume.is_some() {
            let record: SavedCommand =
                read_record(&path, RecordKind::Job, &id, u64::MAX, context.is_some())
                    .map_err(|error| error.to_string())?;
            if record.version != 1
                || record.id != id
                || record.vault_locations != locations
                || record.source != endpoint(source)
                || record.target != endpoint(target)
                || record.source_path != source.path
                || record.target_argument != target.path
                || record.recursive != recursive
                || record.force != force
                || record.moving != moving
                || folders.is_some_and(|folders| folders != record.folders)
            {
                return Err(format!(
                    "Auftrag {id} hat einen abweichenden Quellen-/Ziel-/Policyvertrag"
                ));
            }
            if record.complete {
                return Err(format!("Auftrag {id} ist bereits abgeschlossen"));
            }
            confirm_parent(&record, target)?;
            record.stamp.validate(source, meta)?;
            record
        } else {
            match read_record::<SavedCommand>(
                &path,
                RecordKind::Job,
                &id,
                u64::MAX,
                context.is_some(),
            ) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
                Ok(_) => {
                    return Err("Eine neue Kopie darf keine vorhandene Job-ID überschreiben".into())
                }
            }
            let parent = super::ops::parent_of(&destination)
                .ok_or_else(|| "Ziel hat keinen übergeordneten Ordner".to_string())?;
            let parent_object = crate::vfs::directory_identity(&*target.backend, &parent)
                .ok()
                .flatten();
            let parent_location = parent_object
                .is_none()
                .then(|| location_identity(&*target.backend, &parent))
                .flatten();
            let record = SavedCommand {
                version: 1,
                id: id.clone(),
                source: endpoint(source),
                target: endpoint(target),
                source_path: source.path.clone(),
                vault_locations: locations,
                source_local_root: source
                    .backend
                    .is_local()
                    .then(|| source.backend.root_display()),
                target_argument: target.path.clone(),
                target_local_root: target
                    .backend
                    .is_local()
                    .then(|| target.backend.root_display()),
                destination,
                parent,
                parent_object,
                parent_location,
                recursive,
                force,
                moving,
                folders: folders.unwrap_or(FolderConflict::Merge),
                stamp: SourceStamp::capture(source, meta)?,
                complete: false,
                mutation_started: false,
            };
            write_record(&path, RecordKind::Job, &id, context.as_ref(), &record)
                .map_err(|error| error.to_string())?;
            record
        };
        eprintln!("se: Job-ID {id}");
        Ok(Self {
            path,
            _lock: lock,
            record,
            context,
        })
    }

    pub fn finish(&mut self) -> Result<(), String> {
        self.record.complete = true;
        write_record(
            &self.path,
            RecordKind::Job,
            &self.record.id,
            self.context.as_ref(),
            &self.record,
        )
        .map_err(|error| format!("Job {}: Abschluss speichern: {error}", self.record.id))
    }

    pub fn prepare_alias(&mut self) -> Result<bool, String> {
        let required = self.record.mutation_started;
        if !required {
            self.record.mutation_started = true;
            write_record(
                &self.path,
                RecordKind::Job,
                &self.record.id,
                self.context.as_ref(),
                &self.record,
            )
            .map_err(|error| error.to_string())?;
        }
        Ok(required)
    }
}
