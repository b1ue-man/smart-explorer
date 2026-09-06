use super::engine::{invalid_data, lock, Entry, EntryState, MountEngine};
use super::case_semantics::{identity_key, validate_backend_case_path};
use super::journal::{DeletePhase, PersistedDelete};
use super::metadata_cache::MetadataCache;
use super::metadata_point_cache::MetadataPointCache;
use super::path::{validate_windows_component, PathProjector};
use super::spool::{prepare_spool_root, WholeFileSpool};
use super::startup::validate_backend_root;
use super::types::{BackendRoot, DeleteToken, EntryCondition, MountRuntimeConfig};
use crate::vfs::BackendHandle;
use std::{collections::{HashMap, HashSet}, io, path::Path, sync::{Arc, Mutex, RwLock, atomic::{AtomicBool, AtomicU64, AtomicUsize}}};

impl MountEngine {
    pub(super) fn open_at_root(
        config: MountRuntimeConfig,
        root: BackendRoot,
        backend: BackendHandle,
        spool_root: &Path,
        verify_backend_ancestors: bool,
        validate_remote: bool,
    ) -> io::Result<Self> {
        config.metadata.validate()?;
        config.cache.validate()?;
        let projector = PathProjector::new(root);
        let case_sensitive_paths = backend.case_sensitive_paths(projector.root().as_str());
        if validate_remote {
            validate_backend_root(&*backend, projector.root().as_str())?;
        }
        let spool_root = prepare_spool_root(spool_root)?;
        let (spool, recovered) = WholeFileSpool::open(&spool_root, &config.id)?;
        let spool = Arc::new(spool);
        let mut entries = HashMap::new();
        let retirement_pending = Arc::new(AtomicBool::new(true));
        let mut spool_names = HashSet::new();
        for persisted in recovered.entries.into_values() {
            validate_recovered_path(projector.root().as_str(), &persisted.remote_path, false)?;
            validate_backend_case_path(
                case_sensitive_paths,
                projector.root().as_str(),
                &persisted.remote_path,
            )?;
            if persisted.condition == EntryCondition::Clean && persisted.delete_token.is_none() {
                return Err(invalid_data(
                    "clean journal entry lacks a pending delete transaction",
                ));
            }
            if !spool_names.insert(persisted.spool_name.clone()) {
                return Err(invalid_data("two journal entries reference one spool file"));
            }
            let key = identity_key(case_sensitive_paths, &persisted.remote_path);
            if entries
                .insert(
                    key,
                    Arc::new(Entry {
                        state: Mutex::new(EntryState::from_persisted(persisted)),
                        pins: AtomicUsize::new(0),
                        retirement_pending: Arc::clone(&retirement_pending),
                    }),
                )
                .is_some()
            {
                return Err(invalid_data(
                    "journal contains case-aliasing mount cache entries",
                ));
            }
        }
        let next_delete = recovered
            .deletes
            .keys()
            .copied()
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| invalid_data("delete token space exhausted"))?;
        let mut delete_targets = HashSet::new();
        for (token, delete) in &recovered.deletes {
            validate_recovered_delete(projector.root().as_str(), *token, delete)?;
            validate_backend_case_path(
                case_sensitive_paths,
                projector.root().as_str(),
                &delete.original_path,
            )?;
            if !delete.quarantine_path.is_empty() {
                validate_backend_case_path(
                    case_sensitive_paths,
                    projector.root().as_str(),
                    &delete.quarantine_path,
                )?;
            }
            if !delete_targets.insert(identity_key(case_sensitive_paths, &delete.original_path)) {
                return Err(invalid_data(
                    "journal contains case-aliasing delete transactions",
                ));
            }
        }
        let deletes = recovered
            .deletes
            .into_iter()
            .map(|(token, delete)| (DeleteToken(token), delete))
            .collect::<HashMap<_, _>>();
        let mut namespace_conflicts = HashMap::new();
        for intent in recovered.namespace_conflicts.into_values() {
            let conflict = &intent.conflict;
            validate_recovered_path(projector.root().as_str(), &conflict.path, false)?;
            validate_backend_case_path(
                case_sensitive_paths,
                projector.root().as_str(),
                &conflict.path,
            )?;
            let key = identity_key(case_sensitive_paths, &conflict.path);
            if let Some(source) = intent.source_path.as_deref() {
                validate_recovered_path(projector.root().as_str(), source, false)?;
                validate_backend_case_path(
                    case_sensitive_paths,
                    projector.root().as_str(),
                    source,
                )?;
            }
            if namespace_conflicts.insert(key, intent).is_some() {
                return Err(invalid_data(
                    "journal contains case-aliasing namespace conflicts",
                ));
            }
        }
        for entry in entries.values() {
            let state = lock(&entry.state)?;
            if let Some(token) = state.delete_token {
                let delete = deletes.get(&DeleteToken(token)).ok_or_else(|| {
                    invalid_data("journal entry references an unknown delete token")
                })?;
                if delete.original_path != state.remote_path {
                    return Err(invalid_data(
                        "journal entry and delete transaction target different paths",
                    ));
                }
            }
        }
        let metadata_cache = MetadataCache::new(projector.root().as_str(), case_sensitive_paths);
        let metadata_points = MetadataPointCache::new(case_sensitive_paths);
        let engine = Self {
            config,
            backend,
            case_sensitive_paths,
            verify_backend_ancestors,
            projector,
            spool,
            namespace: RwLock::new(()),
            materializations: Mutex::new(HashMap::new()),
            entries: Mutex::new(entries),
            handles: Mutex::new(HashMap::new()),
            deletes: Mutex::new(deletes),
            namespace_conflicts: Mutex::new(namespace_conflicts),
            metadata_cache,
            metadata_points,
            metadata_epoch: AtomicU64::new(0),
            next_handle: AtomicU64::new(1),
            next_delete: AtomicU64::new(next_delete),
            clean_cache: super::clean_cache::CleanCache::default(),
            cache_space: Arc::new(super::cache_space::CacheSpace::default()),
            detached: Mutex::new(HashMap::new()),
            retirement_pending,
        };
        if validate_remote {
            engine.recover_pending_deletes()?;
        }
        Ok(engine)
    }

}
fn require_below_root(root: &str, path: &str) -> io::Result<()> {
    let valid = path == root
        || (root == "/" && path.starts_with('/') && !path.starts_with("//"))
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'));
    if !valid || path.split('/').any(|part| matches!(part, "." | "..")) {
        return Err(invalid_data(
            "journal path escapes the configured backend root",
        ));
    }
    Ok(())
}

fn validate_recovered_path(root: &str, path: &str, allow_root: bool) -> io::Result<()> {
    require_below_root(root, path)?;
    if path == root {
        return if allow_root {
            Ok(())
        } else {
            Err(invalid_data("journal entry may not target the mount root"))
        };
    }
    let suffix = path
        .strip_prefix(root)
        .ok_or_else(|| invalid_data("journal path is outside the mount root"))?
        .trim_start_matches('/');
    for component in suffix.split('/') {
        validate_windows_component(component).map_err(|error| invalid_data(error.to_string()))?;
    }
    Ok(())
}

fn validate_recovered_delete(root: &str, token: u64, delete: &PersistedDelete) -> io::Result<()> {
    if token == 0 || delete.token != token {
        return Err(invalid_data("delete journal token mismatch"));
    }
    validate_recovered_path(root, &delete.original_path, false)?;
    if delete.phase == DeletePhase::LocalOnly {
        if delete.is_directory || !delete.quarantine_path.is_empty() {
            return Err(invalid_data("invalid local-only delete journal entry"));
        }
        return Ok(());
    }
    validate_recovered_path(root, &delete.quarantine_path, false)?;
    let suffix = delete
        .quarantine_path
        .strip_prefix(&delete.original_path)
        .and_then(|suffix| suffix.strip_prefix(".se-mount-delete-"))
        .ok_or_else(|| invalid_data("delete quarantine is not an exact generated sibling"))?;
    if suffix.len() != 16 || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid_data("delete quarantine suffix is invalid"));
    }
    Ok(())
}
