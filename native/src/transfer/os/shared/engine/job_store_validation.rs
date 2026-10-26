//! Stable endpoint, selected-source and move-result validation.
use std::io;

use super::{
    read_record, record_context, Backend, Endpoint, JobItems, JobRecord, JobView, LocalBackend,
    RecordKind, Side, SourceGuard, StoredEndpoint,
};

pub(in crate::transfer::engine) fn endpoint(side: Side<'_>) -> StoredEndpoint {
    match side {
        Side::Local => StoredEndpoint::of(&Endpoint::Local),
        Side::Remote(backend) => StoredEndpoint {
            local: false,
            namespace: backend.namespace_identity(),
            state: backend.state_identity(),
        },
    }
}

pub(in crate::transfer::engine) fn with_backend<T>(
    side: Side<'_>,
    operation: impl FnOnce(&dyn Backend) -> T,
) -> T {
    match side {
        Side::Local => operation(&LocalBackend::new("/")),
        Side::Remote(backend) => operation(backend),
    }
}

pub(in crate::transfer::engine) fn snapshot(side: Side<'_>, path: &str) -> SourceGuard {
    with_backend(side, |backend| match backend.stat(path) {
        Ok(meta) => SourceGuard {
            path: path.to_string(),
            directory: meta.is_dir,
            link: meta.is_symlink,
            id: meta.id,
            object: if meta.is_dir && !meta.is_symlink {
                crate::vfs::directory_identity(backend, path).ok().flatten()
            } else if !meta.is_symlink && !meta.special {
                crate::vfs::file_identity(backend, path).ok().flatten()
            } else {
                None
            },
            size: meta.size,
            mtime_ms: meta.mtime_ms,
            verified: true,
        },
        Err(_) => SourceGuard {
            path: path.to_string(),
            directory: false,
            link: false,
            id: None,
            object: None,
            size: 0,
            mtime_ms: 0,
            verified: false,
        },
    })
}

pub(super) fn source_paths(items: &JobItems) -> Vec<&str> {
    match items {
        JobItems::Roots { paths, .. } => paths.iter().map(String::as_str).collect(),
        JobItems::Pairs(pairs) => pairs.iter().map(|pair| pair.source.as_str()).collect(),
    }
}

pub(super) fn verify_target(view: &JobView<'_>, record: &JobRecord) -> io::Result<()> {
    if record.target != endpoint(view.target) {
        return Err(io::Error::other(
            "Zielverbindung des gespeicherten Auftrags stimmt nicht überein",
        ));
    }
    if crate::transfer::engine::file_recovery::missing_retry::target(view, record)? {
        return Ok(());
    }
    let actual = with_backend(view.target, |backend| {
        crate::vfs::directory_identity(backend, &record.target_dir)
    })?;
    if record.target_object.is_none() || record.target_object != actual {
        return Err(io::Error::other(
            "Gespeicherter Zielroot ist geändert oder unbestätigt",
        ));
    }
    Ok(())
}

pub(super) fn verify_sources(
    view: &JobView<'_>,
    record: &JobRecord,
    files: &crate::transfer::engine::file_recovery::FileRoots,
) -> io::Result<Vec<crate::transfer::ResolvedRoot>> {
    let mut completed = Vec::new();
    let paths = source_paths(view.items);
    if record.guards.len() != paths.len()
        || record
            .guards
            .iter()
            .zip(&paths)
            .any(|(guard, path)| guard.path != *path)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Gespeicherte Quellbelege decken die Auswahl nicht vollständig ab",
        ));
    }
    for (index, before) in record.guards.iter().enumerate() {
        if !before.verified {
            return Err(io::Error::other(format!(
                "Quellbeleg von Auftrag {} ist unbestätigt",
                record.id
            )));
        }
        let after = snapshot(view.source, &before.path);
        if record.moving && !after.verified {
            if !before.directory {
                let selected = match &record.items {
                    JobItems::Pairs(pairs) => {
                        format!("{}\0{}", pairs[index].source, pairs[index].rel)
                    }
                    _ => before.path.clone(),
                };
                if let Some((_, root)) = files.get(&selected).filter(|(guard, _)| guard == before) {
                    completed.push(root.clone());
                    continue;
                }
            }
            if let Some(relative) = confirmed_move(view, record, before)? {
                completed.push(crate::transfer::ResolvedRoot {
                    source: before.path.clone(),
                    rel: relative,
                });
                continue;
            }
        }
        let same = after.verified
            && before.directory == after.directory
            && before.link == after.link
            && before.id == after.id
            && if before.directory {
                before.object.is_some() && before.object == after.object
                    || before.id.is_some() && before.id == after.id
                    || crate::transfer::engine::file_recovery::missing_retry::live(view)
                        && before.object.is_none()
                        && before.id.is_none()
            } else {
                before.size == after.size
                    && before.mtime_ms == after.mtime_ms
                    && (before.object.is_none() || before.object == after.object)
            };
        if !same {
            return Err(io::Error::other(format!(
                "Quellidentität von Auftrag {} ist geändert oder unbestätigt",
                record.id
            )));
        }
    }
    Ok(completed)
}

pub(super) fn confirmed_move(
    view: &JobView<'_>,
    record: &JobRecord,
    guard: &SourceGuard,
) -> io::Result<Option<String>> {
    let path =
        crate::support_dirs::app_data_file("transfer-aliases").join(format!("{}.json", record.id));
    let context = record_context(&record.vault_locations)?;
    let aliases: crate::transfer::folder_alias::FolderAliases = match read_record(
        &path,
        RecordKind::Alias,
        &record.id,
        u64::MAX,
        context.is_some(),
    ) {
        Ok(aliases) => aliases,
        Err(error)
            if error.kind() == io::ErrorKind::NotFound
                && record.source.local
                && record.target.local
                && record.filter.is_none()
                && record.layout == crate::transfer::Layout::Tree
                && record.folders.for_root(&guard.path)
                    == crate::transfer::FolderConflict::Merge =>
        {
            let JobItems::Roots { base, .. } = &record.items else {
                return Ok(None);
            };
            let relative = crate::transfer::engine_names::root_rel(&guard.path, base.as_deref());
            let actual = with_backend(view.target, |backend| {
                crate::vfs::directory_identity(
                    backend,
                    &crate::transfer::engine_names::join_rel(&record.target_dir, &relative),
                )
            })?;
            return Ok((guard.object.is_some() && guard.object == actual).then_some(relative));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    crate::transfer::engine::folder_alias_store::validate_aliases(&aliases)?;
    if aliases.scope.job != record.id
        || aliases.scope.native_target != record.target.local
        || aliases.scope.vault_locations != record.vault_locations
        || aliases.scope.source_namespace != record.source.namespace
        || aliases.scope.target_namespace != record.target.namespace
        || aliases.scope.target_root != record.target_dir
    {
        return Ok(None);
    }
    let Some(alias) = aliases
        .roots
        .get(&guard.path)
        .filter(|alias| alias.move_source && alias.identity.is_some())
    else {
        return Ok(None);
    };
    if guard.object != alias.identity {
        return Ok(None);
    }
    with_backend(view.target, |backend| {
        crate::vfs::directory_identity(
            backend,
            &crate::transfer::engine_names::join_rel(&record.target_dir, &alias.relative),
        )
    })
    .map(|identity| (identity == alias.identity).then(|| alias.relative.clone()))
}
