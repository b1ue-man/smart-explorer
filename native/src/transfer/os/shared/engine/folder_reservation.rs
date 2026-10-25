//! Exclusive selected-root reservation with a durable pre-publication receipt.
use super::folder_alias_store::AliasStore;
use super::super::super::folder_alias::{FolderAlias, FolderAliasScope};
use super::super::engine_names::{join_rel, parent_rel};
use super::view::Side;
use crate::vfs::{Backend, LocalBackend};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[path = "folder_move.rs"]
mod moves;
pub(crate) use moves::{reserve_named_move, confirmed_named_move};

pub(crate) struct Reservations {
    store: Mutex<AliasStore>,
}

impl Reservations {
    pub fn saved_move(&self, source: &str) -> bool {
        self.store.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).data.roots.get(source).is_some_and(|saved| saved.move_source)
    }
    pub fn saved_relative(&self, source: &str) -> Option<String> {
        self.store.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).data.roots.get(source).map(|saved| saved.relative.clone())
    }

    pub fn confirm_resume(&self, target: Side<'_>, root: &str, source: &str, relative: &str) -> io::Result<Option<bool>> {
        let local = LocalBackend::new("/");
        let backend: &dyn Backend = match target { Side::Local => &local, Side::Remote(backend) => backend };
        let mut store = self.store.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(saved) = store.data.roots.get(source).cloned() else { return Ok(None); };
        if saved.relative != relative { return Err(io::Error::other("Wiederaufnahmeziel weicht vom gespeicherten Alias ab")); }
        let identity = saved.identity.as_ref().ok_or_else(|| io::Error::other("Gespeicherter Alias ist unbestätigt"))?;
        if crate::vfs::directory_identity(backend, &join_rel(root, relative))?.as_ref() != Some(identity) { return Err(io::Error::other("Gespeicherter Zielalias wurde ersetzt oder fehlt")); }
        let mut confirmed = saved.clone(); confirmed.published = true;
        store.data.roots.insert(source.to_string(), confirmed); store.save()?;
        Ok(Some(saved.move_source))
    }

    pub fn open(scope: FolderAliasScope, recovery: crate::transfer::RecoveryMode, required: bool) -> io::Result<Self> {
        Ok(Self { store: Mutex::new(AliasStore::open(scope, recovery, required)?) })
    }

    pub fn reserve(&self, target: Side<'_>, root: &str, source: &str, wanted: &str, renumber: bool, cancel: &AtomicBool) -> io::Result<String> {
        let local = LocalBackend::new("/");
        let backend: &dyn Backend = match target { Side::Local => &local, Side::Remote(backend) => backend };
        let mut store = self.store.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(saved) = store.data.roots.get(source).cloned() {
            if saved.wanted != wanted { return Err(io::Error::other("Gespeicherter Quellordner hat einen abweichenden Zielvertrag")); }
            return Self::publish(backend, root, &mut store, source, saved);
        }
        let parent = parent_rel(wanted).unwrap_or("");
        let parent_path = join_rel(root, parent);
        let direct = crate::vfs::directory_identity(backend, &parent_path)?.is_none();
        let name = wanted.rsplit('/').next().unwrap_or(wanted);
        // mkdir arbitrates; dots remain literal directory-name characters.
        for index in 1..=crate::vfs::remote_util::REMOTE_UNIQUE_ATTEMPTS {
            if cancel.load(Ordering::Acquire) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, super::super::cancel::CANCELED_ERROR));
            }
            let name = if index == 1 { name.to_string() } else { format!("{name} ({index})") };
            let relative = join_rel(parent, &name);
            match backend.stat(&join_rel(root, &relative)) {
                Ok(_) if renumber => continue,
                Ok(_) => return Err(io::Error::new(io::ErrorKind::AlreadyExists, "Zielordner existiert bereits")),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {},
                Err(error) => return Err(error),
            }
            let stage = if direct { String::new() } else { join_rel(parent, &format!(".se-folder-{}", super::new_job_id())) };
            let mut saved = FolderAlias { wanted: wanted.to_string(), relative, identity: None, stage, published: false, move_source: false, direct };
            store.data.roots.insert(source.to_string(), saved.clone());
            store.save()?;
            if direct {
                match crate::vfs::create_dir_exclusive(backend, &join_rel(root, &saved.relative)) {
                    Ok(()) => {
                        saved.published = true;
                        store.data.roots.insert(source.to_string(), saved.clone()); store.save()?;
                        saved.identity = crate::vfs::directory_identity(backend, &join_rel(root, &saved.relative))?;
                        store.data.roots.insert(source.to_string(), saved.clone()); store.save()?;
                        return Ok(saved.relative);
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists && renumber => {
                        store.data.roots.remove(source); store.save()?; continue;
                    }
                    Err(error) => return Err(error),
                }
            }
            let stage_path = join_rel(root, &saved.stage);
            crate::vfs::create_dir_exclusive(backend, &stage_path)?;
            saved.identity = crate::vfs::directory_identity(backend, &stage_path)?;
            if saved.identity.is_none() {
                // This call created the empty stage; no target was published.
                backend.remove_dir(&stage_path)?;
                store.data.roots.remove(source);
                store.save()?;
                return Err(io::Error::new(io::ErrorKind::Unsupported, "Ziel bestätigt keine dauerhafte Ordneridentität"));
            }
            store.data.roots.insert(source.to_string(), saved.clone());
            store.save()?;
            match Self::publish(backend, root, &mut store, source, saved.clone()) {
                Ok(relative) => return Ok(relative),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    // Only a definite NoReplace collision permits renumbering.
                    if crate::vfs::directory_identity(backend, &stage_path)? == saved.identity {
                        backend.remove_dir(&stage_path)?;
                    } else {
                        return Err(io::Error::other("Identität der Ordnerreservierung hat sich geändert"));
                    }
                    store.data.roots.remove(source);
                    store.save()?;
                    if !renumber { return Err(error); }
                }
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "Kein freier Ordnername im Ziel"))
    }

    fn publish(backend: &dyn Backend, root: &str, store: &mut AliasStore, source: &str, mut saved: FolderAlias) -> io::Result<String> {
        let target = join_rel(root, &saved.relative);
        if saved.move_source { return Err(io::Error::other("Movealias verlangt den gespeicherten Move-Wiederaufnahmeweg")); }
        if saved.direct && saved.identity.is_none() {
            if saved.published { return Err(io::Error::other("Die frühere Ordneranlage wurde bestätigt. Für einen erneuten Lauf fehlt eine dauerhafte Zielidentität; der Auftrag bleibt unbestätigt")); }
            match backend.stat(&target) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    crate::vfs::create_dir_exclusive(backend, &target)?;
                    saved.published = true;
                    store.data.roots.insert(source.to_string(), saved.clone()); store.save()?;
                    saved.identity = crate::vfs::directory_identity(backend, &target)?;
                    store.data.roots.insert(source.to_string(), saved.clone()); store.save()?;
                    return Ok(saved.relative);
                }
                Err(error) => return Err(error),
                Ok(_) => return Err(io::Error::other("Frühere Ordneranlage ist nicht bestätigt; derselbe Zielname bleibt geschützt und wird nicht übernommen oder neu vergeben")),
            }
        }
        if saved.identity.is_none() {
            let stage = join_rel(root, &saved.stage);
            match crate::vfs::directory_identity(backend, &stage) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    crate::vfs::create_dir_exclusive(backend, &stage)?;
                    saved.identity = crate::vfs::directory_identity(backend, &stage)?;
                    store.data.roots.insert(source.to_string(), saved.clone()); store.save()?;
                }
                Err(error) => return Err(error),
                Ok(_) => return Err(io::Error::other("Vorhandene Stage besitzt keinen gespeicherten Eigentumsbeleg")),
            }
        }
        let identity = saved.identity.as_ref().ok_or_else(|| io::Error::other(
            "Unbestätigte Ordnerreservierung nach Unterbrechung; derselbe Zielalias bleibt geschützt"
        ))?;
        match crate::vfs::directory_identity(backend, &target) {
            Ok(Some(found)) if &found == identity => {
                saved.published = true;
                store.data.roots.insert(source.to_string(), saved.clone()); store.save()?;
                return Ok(saved.relative);
            }
            Ok(_) if saved.published => return Err(io::Error::other("Gespeicherter Zielordner wurde ersetzt")),
            Err(error) if saved.published && error.kind() == io::ErrorKind::NotFound => return Err(io::Error::other("Gespeicherter Zielordner fehlt")),
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        let stage = join_rel(root, &saved.stage);
        if crate::vfs::directory_identity(backend, &stage)?.as_ref() != Some(identity) {
            return Err(io::Error::other("Gespeicherte Ordnerreservierung ist nicht mehr dieselbe"));
        }
        backend.rename_no_replace(&stage, &target)?;
        if crate::vfs::directory_identity(backend, &target)?.as_ref() != Some(identity) {
            return Err(io::Error::other("Zielordneridentität nach Veröffentlichung unbestätigt"));
        }
        let _ = crate::vfs::confirm_namespace(backend, parent_rel(&target).unwrap_or(root))?;
        saved.published = true;
        store.data.roots.insert(source.to_string(), saved.clone());
        store.save()?;
        Ok(saved.relative)
    }
}

/// Terminal/MCP use the exact destination selected by their established
/// path resolver. The returned guard must live until copy/move completes.
pub(crate) fn reserve_named_folder(
    source: &dyn Backend,
    source_path: &str,
    target: &dyn Backend,
    destination: &str,
    renumber: bool,
    job_id: &str,
    recovery: crate::transfer::RecoveryMode,
    required: bool,
    cancel: &AtomicBool,
) -> io::Result<(String, Reservations)> {
    let parent = super::super::engine_names::parent_path(destination);
    target.mkdir_all(&parent)?;
    let reservations = Reservations::open(FolderAliasScope {
        job: job_id.to_string(),
        source_namespace: source.namespace_identity(),
        target_namespace: target.namespace_identity(),
        target_root: parent.clone(), native_target: target.is_local(),
        vault_locations: super::record_io::record_locations(Side::Remote(source), Side::Remote(target), &[source_path], &parent)?,
    }, recovery, required)?;
    let relative = reservations.reserve(Side::Remote(target), &parent, source_path, super::super::engine_names::base_name(destination), renumber, cancel)?;
    Ok((join_rel(&parent, &relative), reservations))
}
