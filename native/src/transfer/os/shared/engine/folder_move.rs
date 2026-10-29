//! Whole-folder NoReplace moves retain the source object and its OS metadata.
use super::Reservations;
use crate::transfer::engine_names::{join_rel, parent_rel};
use crate::transfer::folder_alias::{FolderAlias, FolderAliasScope};
use crate::transfer::Side;
use crate::vfs::{Backend, LocalBackend};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

impl Reservations {
    /// Preserve the original directory object on same-volume moves. A
    /// cross-volume fallback reserves the target for the existing file move.
    pub fn move_root(
        &self,
        root: &str,
        source: &str,
        wanted: &str,
        renumber: bool,
        cancel: &AtomicBool,
    ) -> io::Result<(String, bool)> {
        let backend = LocalBackend::new("/");
        let mut store = self
            .store
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(saved) = store.data.roots.get(source).cloned() {
            if saved.wanted != wanted {
                return Err(io::Error::other(
                    "Gespeicherter Move hat einen abweichenden Zielvertrag",
                ));
            }
            if !saved.move_source {
                let relative = Self::publish(&backend, root, &mut store, source, saved, cancel)?;
                return Ok((relative, false));
            }
            let target = join_rel(root, &saved.relative);
            match crate::vfs::directory_identity(&backend, &target) {
                Ok(identity) if saved.identity.is_some() && identity == saved.identity => {
                    let mut confirmed = saved.clone();
                    confirmed.published = true;
                    store.data.roots.insert(source.to_string(), confirmed);
                    store.save()?;
                    return Ok((saved.relative, true));
                }
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
            if saved.published {
                return Err(io::Error::other("Veröffentlichtes Moveziel wurde ersetzt"));
            }
        }
        let source_path = crate::transfer::walk_listers::native(source);
        let source_identity =
            crate::vfs::directory_identity(&backend, source)?.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Quellordneridentität für Move ist unbekannt",
                )
            })?;
        let parent = parent_rel(wanted).unwrap_or("");
        let name = crate::transfer::engine_names::base_name(wanted);
        let resumed = store.data.roots.get(source).cloned();
        for index in 1..=crate::vfs::remote_util::REMOTE_UNIQUE_ATTEMPTS {
            if cancel.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    crate::transfer::cancel::CANCELED_ERROR,
                ));
            }
            let relative = match &resumed {
                Some(saved) if index == 1 => {
                    if saved.identity.as_ref() != Some(&source_identity) {
                        return Err(io::Error::other(
                            "Quellordner des unterbrochenen Move wurde ersetzt",
                        ));
                    }
                    saved.relative.clone()
                }
                _ => super::relative_child(
                    parent,
                    &if index == 1 {
                        name.to_string()
                    } else {
                        format!("{name} ({index})")
                    },
                ),
            };
            let mut saved = FolderAlias {
                wanted: wanted.to_string(),
                relative: relative.clone(),
                identity: Some(source_identity.clone()),
                stage: String::new(),
                published: false,
                move_source: true,
                direct: false,
            };
            store.data.roots.insert(source.to_string(), saved.clone());
            store.save()?;
            let target = join_rel(root, &relative);
            match crate::copy::move_folder(
                &source_path,
                &crate::transfer::walk_listers::native(&target),
            ) {
                Ok(()) => {
                    if crate::vfs::directory_identity(&backend, &target)?.as_ref()
                        != Some(&source_identity)
                    {
                        return Err(io::Error::other(
                            "Verschobener Zielordner ist nicht die bestätigte Quelle",
                        ));
                    }
                    saved.published = true;
                    store.data.roots.insert(source.to_string(), saved);
                    store.save()?;
                    return Ok((relative, true));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && renumber => {
                    if resumed.is_some() {
                        return Err(io::Error::other("Gespeicherter Movealias ist inzwischen belegt; keine neue Nummer vergeben"));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
                    store.data.roots.remove(source);
                    store.save()?;
                    drop(store);
                    let relative =
                        self.reserve(Side::Local, root, source, wanted, renumber, cancel)?;
                    return Ok((relative, false));
                }
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Kein freier Moveordnername im Ziel",
        ))
    }
}

pub(crate) fn reserve_named_move(
    source: &dyn Backend,
    source_path: &str,
    target: &dyn Backend,
    destination: &str,
    renumber: bool,
    job_id: &str,
    recovery: crate::transfer::RecoveryMode,
    required: bool,
    cancel: &AtomicBool,
) -> io::Result<(String, Reservations, bool)> {
    let parent = crate::transfer::engine_names::parent_path(destination);
    target.mkdir_all(&parent)?;
    let reservations = Reservations::open(
        FolderAliasScope {
            job: job_id.to_string(),
            source_namespace: source.namespace_identity(),
            target_namespace: target.namespace_identity(),
            target_root: parent.clone(),
            native_target: target.is_local(),
            vault_locations: crate::transfer::engine::record_io::record_locations(
                Side::Remote(source),
                Side::Remote(target),
                &[source_path],
                &parent,
            )?,
        },
        recovery,
        required,
    )?;
    let (relative, moved) = reservations.move_root(
        &parent,
        source_path,
        crate::transfer::engine_names::base_name(destination),
        renumber,
        cancel,
    )?;
    Ok((join_rel(&parent, &relative), reservations, moved))
}

pub(crate) fn confirmed_named_move(
    source: &dyn Backend,
    source_path: &str,
    target: &dyn Backend,
    destination: &str,
    job_id: &str,
    expected: &str,
) -> io::Result<Option<String>> {
    let parent = crate::transfer::engine_names::parent_path(destination);
    let reservations = Reservations::open(
        FolderAliasScope {
            job: job_id.to_string(),
            source_namespace: source.namespace_identity(),
            target_namespace: target.namespace_identity(),
            target_root: parent.clone(),
            native_target: target.is_local(),
            vault_locations: crate::transfer::engine::record_io::record_locations(
                Side::Remote(source),
                Side::Remote(target),
                &[source_path],
                &parent,
            )?,
        },
        crate::transfer::RecoveryMode::Resume,
        true,
    )?;
    let store = reservations
        .store
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(saved) = store.data.roots.get(source_path) else {
        return Ok(None);
    };
    if !saved.move_source
        || saved.wanted != crate::transfer::engine_names::base_name(destination)
        || saved.identity.as_deref() != Some(expected)
    {
        return Err(io::Error::other(
            "Gespeicherter Movebeleg passt nicht zum Auftrag",
        ));
    }
    let actual = join_rel(&parent, &saved.relative);
    if crate::vfs::directory_identity(target, &actual)?.as_deref() != Some(expected) {
        return Err(io::Error::other(
            "Verschobener Zielalias ist geändert oder unbestätigt",
        ));
    }
    Ok(Some(actual))
}
