//! The comparison mode of a vault sync job (vault plan C6, VLT-M8, U26-2 to
//! U26-5): the engine's preview of the encrypted storage of both sides
//! (`preview_vault_job_in`, with the rules of the vault job) is mapped to
//! cleartext entries in the groups of C6.2. The key of one side opens the
//! names (the sessions of one vault share one key, R7); without an unlocked
//! side the comparison answers `vault_locked`. Nothing is changed here, and
//! nothing leaves the process: the state lives in `compare_cache.rs`.
use std::collections::HashMap;
use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

use crate::bisync::Sig;
use crate::cryptomator::{cleartext_file_size, CipherCombo};
use crate::syncjobs::SyncJob;
use crate::vault::core::compare_model::{
    CompareGroup, CompareItem, CompareView, ItemKind, SideState,
};
use crate::vault::core::path_map::storage_path;
use crate::vault::core::{vault_locked, VaultSession};

use super::compare_cache::{cache, CompareState, Dropper, Held, SideIo};
use super::compare_items::file_items;
use super::dir_map::DirMap;
use super::open::{unlocked_session, ResolveLocation};
use super::registry_store::RegistryStore;
use super::sync_job::{prepare, preview_vault_job_in};

/// The vault job `selector` names: its ID, its exact name or an ID prefix
/// of at least four characters, among the jobs of `vault-jobs/`.
pub(crate) fn find_vault_job(selector: &str) -> io::Result<SyncJob> {
    let selector = selector.trim();
    let report = crate::syncjobs::load_report()?;
    let jobs: Vec<SyncJob> = report.jobs.into_iter().filter(SyncJob::is_vault).collect();
    let rules: [&dyn Fn(&SyncJob) -> bool; 3] = [
        &|job: &SyncJob| job.id == selector,
        &|job: &SyncJob| job.name == selector,
        &|job: &SyncJob| selector.len() >= 4 && job.id.starts_with(selector),
    ];
    for rule in rules {
        let found: Vec<&SyncJob> = jobs.iter().filter(|job| rule(job)).collect();
        match found.as_slice() {
            [] => continue,
            [job] => return Ok((*job).clone()),
            many => {
                let ids: Vec<&str> = many.iter().map(|job| job.id.as_str()).collect();
                return Err(invalid(format!(
                    "Mehrdeutig: {} Tresor-Jobs passen zu „{selector}“ ({})",
                    many.len(),
                    ids.join(", ")
                )));
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("Unbekannter Tresor-Job: {selector}"),
    ))
}

pub(super) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// The session whose key serves `job`: side A first, then side B.
pub(super) fn key_session(job: &SyncJob) -> io::Result<Arc<VaultSession>> {
    [&job.vault_site_a, &job.vault_site_b]
        .into_iter()
        .find_map(|site| unlocked_session(site))
        .ok_or_else(|| vault_locked(&job.vault_site_a))
}

/// The signature the engine would record for the file at `rel` on `side`;
/// `None` when the file is absent.
pub(super) fn signature_of(side: &SideIo, rel: &str) -> io::Result<Option<Sig>> {
    let backend = side.backend.as_ref();
    let path = crate::vfs::sync_path(backend, &side.root, rel)?;
    match crate::vfs::sync_stat(backend, &path) {
        Ok(meta) if meta.is_dir || meta.is_symlink || meta.special => {
            Err(invalid("Eintrag ist keine gewöhnliche Datei"))
        }
        Ok(meta) => Ok(Some(Sig {
            size: meta.size,
            mtime_ms: meta.mtime_ms,
            hash: 0,
        })),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn clear_size(size: u64, combo: CipherCombo) -> Option<u64> {
    cleartext_file_size(size, combo).ok()
}

pub(super) fn side_state(
    side: &SideIo,
    rel: &str,
    kind: ItemKind,
    combo: CipherCombo,
) -> SideState {
    match side.backend.stat(&storage_path(&side.root, rel)) {
        Ok(meta) => SideState {
            exists: true,
            size: (kind == ItemKind::File && !meta.is_dir)
                .then(|| clear_size(meta.size, combo))
                .flatten(),
            mtime_ms: Some(meta.mtime_ms),
            location: side.label.clone(),
        },
        Err(_) => SideState::absent(&side.label),
    }
}

/// The first 16 lower-case hex digits of
/// `SHA-256("se-vault-item\0" ‖ job ‖ "\0" ‖ kind ‖ "\0" ‖ identity)`.
pub(crate) fn item_id(job: &str, kind: &str, identity: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"se-vault-item\0");
    hasher.update(job.as_bytes());
    hasher.update(b"\0");
    hasher.update(kind.as_bytes());
    hasher.update(b"\0");
    hasher.update(identity.as_bytes());
    let digest = hasher.finalize();
    data_encoding::HEXLOWER.encode(&digest[..8])
}

/// Orders the entries (group, path, note, id) and makes the IDs unique:
/// the second entry of one ID gets `-2`, the third `-3` (U26-4).
pub(super) fn order_and_number(helds: &mut [Held]) {
    helds.sort_by(|left, right| {
        let (l, r) = (&left.item, &right.item);
        l.group
            .cmp(&r.group)
            .then(l.path.is_empty().cmp(&r.path.is_empty()))
            .then(l.path.cmp(&r.path))
            .then(l.note.cmp(&r.note))
            .then(l.id.cmp(&r.id))
    });
    let mut seen: HashMap<String, usize> = HashMap::new();
    for held in helds.iter_mut() {
        let count = seen.entry(held.item.id.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            held.item.id = format!("{}-{}", held.item.id, count);
        }
    }
}

/// The groups a request names: none means every group but `structure`.
fn wanted_groups(groups: &[CompareGroup]) -> Vec<CompareGroup> {
    if groups.is_empty() {
        CompareGroup::DEFAULT.to_vec()
    } else {
        groups.to_vec()
    }
}

/// The view of a comparison: its listing with the sides and the counts.
pub(super) fn view_of(state: &CompareState) -> CompareView {
    let side = |index: usize| {
        state
            .sides
            .get(index)
            .map_or(("", ""), |side| (side.site.as_str(), side.label.as_str()))
    };
    CompareView::new(
        &state.key,
        [side(0), side(1)],
        state.revision,
        state.listing.as_ref().clone(),
        state.unavailable.clone(),
    )
}

/// The listing of the comparison `list` (a job, or `check:<site>`) of the
/// number `revision`: the pages of a long answer read it. `compare_stale`
/// when the comparison is gone or a newer one replaced it.
pub(crate) fn compare_snapshot(list: &str, revision: u64) -> io::Result<Arc<Vec<CompareItem>>> {
    cache().listing(list, revision)
}

/// Compares the vault job `job`: `groups` are the groups to list (empty:
/// every group but `structure`). `ErrorKind::Interrupted` when `cancel` is
/// set, `vault_locked` without an unlocked side.
pub(crate) fn compare_job_in(
    store: &RegistryStore,
    resolve: ResolveLocation<'_>,
    job: &SyncJob,
    groups: &[CompareGroup],
    cancel: &AtomicBool,
) -> io::Result<CompareView> {
    if !job.is_vault() {
        return Err(invalid("Das ist kein Tresor-Job"));
    }
    let session = key_session(job)?;
    let sides = prepare(store, resolve, job)
        .map_err(|outcome| io::Error::other(outcome.unavailable.unwrap_or_default()))?;
    let preview = preview_vault_job_in(store, resolve, job, cancel);
    if let Some(reason) = preview
        .unavailable
        .clone()
        .or_else(|| preview.preview.error.clone())
    {
        return Err(io::Error::other(reason));
    }
    let wanted = wanted_groups(groups);
    let labels = [
        sides.records[0].display_label().to_string(),
        sides.records[1].display_label().to_string(),
    ];
    let ios = vec![
        SideIo {
            backend: Arc::clone(&sides.a.0),
            root: sides.a.1.clone(),
            site: job.vault_site_a.clone(),
            label: labels[0].clone(),
        },
        SideIo {
            backend: Arc::clone(&sides.b.0),
            root: sides.b.1.clone(),
            site: job.vault_site_b.clone(),
            label: labels[1].clone(),
        },
    ];
    let maps = vec![
        DirMap::build(ios[0].backend.as_ref(), &ios[0].root, &session, cancel)?,
        DirMap::build(ios[1].backend.as_ref(), &ios[1].root, &session, cancel)?,
    ];
    let mut helds = file_items(job, &preview, &ios, &maps, &session);
    if wanted.contains(&CompareGroup::Structure) {
        helds.extend(super::health::job_findings(&job.id, &ios, &maps, &session));
    }
    order_and_number(&mut helds);
    let listing: Vec<CompareItem> = helds
        .iter()
        .filter(|held| wanted.contains(&held.item.group))
        .map(|held| held.item.clone())
        .collect();
    let unavailable = preview
        .preview
        .blocked
        .as_ref()
        .map(|block| block.message());
    let site = session.site().to_string();
    let state = cache().put(CompareState {
        key: job.id.clone(),
        revision: cache().next_revision(),
        site: site.clone(),
        session,
        job: Some(job.clone()),
        unavailable,
        applied: Mutex::new(vec![false; preview.preview.actions.len()]),
        preview: Some(Mutex::new(preview)),
        sides: ios,
        maps,
        items: Mutex::new(helds),
        listing: Arc::new(listing),
        listener: Arc::new(Dropper { site }),
    });
    Ok(view_of(&state))
}

/// `compare_job_in` with the vault list and the resolution of this
/// installation; `selector` names the job (ID, name, ID prefix).
pub(crate) fn compare_job(
    selector: &str,
    groups: &[CompareGroup],
    cancel: &AtomicBool,
) -> io::Result<CompareView> {
    let job = find_vault_job(selector)?;
    let resolve = |location: &str| crate::connect::resolve_endpoint(location);
    compare_job_in(
        &RegistryStore::open_default(),
        &resolve,
        &job,
        groups,
        cancel,
    )
}
