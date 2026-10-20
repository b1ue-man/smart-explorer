//! The actions on the entries of a comparison (vault plan C6.4, U26-5):
//! „Fassung A/B behalten“, „Jetzt übertragen“, „Löschung übernehmen“,
//! „Wiederherstellen“ and the collective action, all through the engine
//! (`resolve_recorded` and `apply_preview_action` take the pair lock
//! themselves and record the baseline); „Beide behalten“, „Zusammenführen“,
//! the folder actions and the structure fixes are in their own files. An
//! action never runs on a comparison that is gone (`compare_stale`) or whose
//! key side is locked (`vault_locked`); a vault job never resolves on its
//! own, so nothing here runs without a call of a surface.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{MutexGuard, PoisonError};

use crate::bisync::{pair_lock_id, Action, Conflict, PairLock};
use crate::syncjobs::SyncJob;
use crate::vault::core::compare_model::{CompareGroup, ItemAction, ItemKind};
use crate::vault::core::health_model::removes_cipher_files;

use super::compare::{find_vault_job, invalid};
use super::compare_cache::{cache, site_key, stale, CompareState, Payload, SideIo};
use super::compare_engine::{confirm_delete, keep, restore, transfer};
use super::open::ResolveLocation;
use super::registry_store::RegistryStore;
use super::sync_job::VaultPreview;

/// What the actions need besides the comparison: the vault list and the
/// resolution of locations (a new preview, the jobs that use a site).
pub(crate) struct ActionEnv<'a> {
    pub(crate) store: &'a RegistryStore,
    pub(crate) resolve: ResolveLocation<'a>,
}

/// One request to `resolve_item`.
pub(crate) struct ResolveRequest<'a> {
    /// A vault job (ID, name, ID prefix) or a site checked before.
    pub(crate) job: &'a str,
    pub(crate) item: Option<&'a str>,
    /// The wire key of the action (`keep-a`, `transfer`, …).
    pub(crate) action: &'a str,
    /// The collective action on every conflict that has two files.
    pub(crate) all: bool,
    /// The user confirmed the question the action asks.
    pub(crate) confirmed: bool,
}

/// What `resolve_item` did.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ResolveReport {
    pub(crate) item: Option<String>,
    pub(crate) resolved: u32,
    /// Entries a collective action could not resolve, with the reason.
    pub(crate) failed: Vec<(String, String)>,
}

/// The cache key a job or site selector stands for.
pub(super) fn state_key(selector: &str) -> String {
    if cache().has(selector) {
        return selector.to_string();
    }
    if let Ok(job) = find_vault_job(selector) {
        return job.id;
    }
    if let Ok(site) = RegistryStore::open_default().resolve(selector) {
        return site_key(&site.id);
    }
    selector.to_string()
}

/// Marks the entry `item` of `state` as done.
pub(super) fn mark_done(state: &CompareState, item: &str) {
    for held in state.items().iter_mut() {
        if held.item.id == item {
            held.done = true;
        }
    }
}

fn mark_done_index(state: &CompareState, index: usize) {
    if let Some(held) = state.items().get_mut(index) {
        held.done = true;
    }
}

/// The two sides of a job comparison.
pub(super) fn pair(state: &CompareState) -> io::Result<(&SideIo, &SideIo)> {
    match state.sides.as_slice() {
        [a, b] => Ok((a, b)),
        _ => Err(invalid(
            "Diese Aktion gehört zu einem Vergleich von zwei Seiten",
        )),
    }
}

pub(super) fn job_of(state: &CompareState) -> io::Result<&SyncJob> {
    state
        .job
        .as_ref()
        .ok_or_else(|| invalid("Der Vergleich gehört zu keinem Tresor-Job"))
}

pub(super) fn lock_preview(state: &CompareState) -> io::Result<MutexGuard<'_, VaultPreview>> {
    state
        .preview
        .as_ref()
        .map(|preview| preview.lock().unwrap_or_else(PoisonError::into_inner))
        .ok_or_else(|| invalid("Der Vergleich gehört zu keinem Tresor-Job"))
}

/// The planned actions, the conflicts and the cipher files of an entry.
pub(super) fn files_of(
    state: &CompareState,
    index: usize,
) -> io::Result<(Vec<Action>, Vec<Conflict>, Vec<String>)> {
    match state.items().get(index).map(|held| &held.payload) {
        Some(Payload::Files {
            actions,
            conflicts,
            rels,
            ..
        }) => Ok((actions.clone(), conflicts.clone(), rels.clone())),
        Some(Payload::Finding { .. }) => Err(invalid("Dieser Eintrag ist ein Befund der Prüfung")),
        None => Err(stale()),
    }
}

fn confirmation(action: ItemAction, group: CompareGroup, confirmed: bool) -> io::Result<()> {
    let question = if action == ItemAction::ConfirmDelete && group == CompareGroup::Deferred {
        Some(
            "Die Löschung betrifft den Inhalt eines Ordners, dessen Verzeichnis-ID-Sicherung \
             auf einer Seite fehlt. Trotzdem übernehmen?"
                .to_string(),
        )
    } else if action == ItemAction::MergeFolders {
        Some(format!(
            "Der Inhalt des Ordners von Seite B wird in den Ordner von Seite A übernommen und \
             dort entfernt; eine Kopie des Chiffrats bleibt in {}.",
            super::compare_files::removed_root().display()
        ))
    } else if removes_cipher_files(action) {
        Some(format!(
            "Dieser Eintrag ist beschädigt oder doppelt. Er wird entfernt; eine Kopie des \
             Chiffrats bleibt in {}.",
            super::compare_files::removed_root().display()
        ))
    } else {
        None
    };
    match question {
        Some(text) if !confirmed => Err(io::Error::new(io::ErrorKind::PermissionDenied, text)),
        _ => Ok(()),
    }
}

/// Runs one action on one entry (`Resolve`).
pub(crate) fn resolve_item_in(
    env: &ActionEnv<'_>,
    request: ResolveRequest<'_>,
    cancel: &AtomicBool,
) -> io::Result<ResolveReport> {
    let state = cache().get(&state_key(request.job))?;
    state.check_unlocked()?;
    if request.all {
        return resolve_all(&state, request.action, cancel);
    }
    let id = request
        .item
        .ok_or_else(|| invalid("Eintragsnummer fehlt"))?;
    let (index, kind, group, allowed) = {
        let items = state.items();
        let (index, held) = items
            .iter()
            .enumerate()
            .find(|(_, held)| held.item.id == id && !held.done)
            .ok_or_else(stale)?;
        (
            index,
            held.item.kind,
            held.item.group,
            held.item.actions.clone(),
        )
    };
    let action = ItemAction::from_key(request.action, kind)
        .ok_or_else(|| invalid(format!("Unbekannte Aktion: {}", request.action)))?;
    if !allowed.contains(&action) {
        return Err(invalid(format!(
            "Die Aktion {} ist für diesen Eintrag nicht möglich",
            request.action
        )));
    }
    confirmation(action, group, request.confirmed)?;
    run_action(env, &state, index, action, cancel)?;
    mark_done_index(&state, index);
    Ok(ResolveReport {
        item: Some(id.to_string()),
        resolved: 1,
        failed: Vec::new(),
    })
}

/// `resolve_item_in` with the vault list and resolution of this
/// installation.
pub(crate) fn resolve_item(
    request: ResolveRequest<'_>,
    cancel: &AtomicBool,
) -> io::Result<ResolveReport> {
    let store = RegistryStore::open_default();
    let resolve = |location: &str| crate::connect::resolve_endpoint(location);
    resolve_item_in(
        &ActionEnv {
            store: &store,
            resolve: &resolve,
        },
        request,
        cancel,
    )
}

fn run_action(
    env: &ActionEnv<'_>,
    state: &CompareState,
    index: usize,
    action: ItemAction,
    cancel: &AtomicBool,
) -> io::Result<()> {
    match action {
        ItemAction::KeepA | ItemAction::KeepVersionA => keep(state, index, true, cancel),
        ItemAction::KeepB | ItemAction::KeepVersionB => keep(state, index, false, cancel),
        ItemAction::Transfer => transfer(state, index, cancel),
        ItemAction::ConfirmDelete => confirm_delete(state, index, cancel),
        ItemAction::Restore => restore(state, index, cancel),
        ItemAction::KeepBoth => super::compare_keep_both::keep_both(env, state, index, cancel),
        ItemAction::MergeFolders => {
            super::compare_folders::merge_folders(env, state, index, cancel)
        }
        ItemAction::KeepFoldersBoth => {
            super::compare_folders::keep_folders_both(env, state, index, cancel)
        }
        ItemAction::Merge => Err(invalid(
            "Zusammenführen: erst den Text holen (CompareText), dann den Text senden (MergeApply)",
        )),
        _ => super::health_fixes::fix(env, state, index, action, cancel),
    }
}

/// The collective action: every conflict with two files, one side wins.
fn resolve_all(
    state: &CompareState,
    action: &str,
    cancel: &AtomicBool,
) -> io::Result<ResolveReport> {
    let keep_a = match action {
        "keep-a" => true,
        "keep-b" => false,
        _ => return Err(invalid("Sammelaktionen kennen nur keep-a und keep-b")),
    };
    let targets: Vec<(usize, String)> = state
        .items()
        .iter()
        .enumerate()
        .filter(|(_, held)| {
            !held.done
                && held.item.group == CompareGroup::Conflict
                && held.item.kind == ItemKind::File
                && held.item.a.exists
                && held.item.b.exists
        })
        .map(|(index, held)| (index, held.item.id.clone()))
        .collect();
    let mut report = ResolveReport::default();
    for (index, id) in targets {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        match keep(state, index, keep_a, cancel) {
            Ok(()) => {
                mark_done_index(state, index);
                report.resolved += 1;
            }
            Err(error) => report.failed.push((id, error.to_string())),
        }
    }
    Ok(report)
}

/// Locks the pair of the comparison (a job) or of every vault job that uses
/// the compared site (a check), for actions that change cipher files
/// themselves; `ErrorKind::WouldBlock` while a run of the pair is going.
pub(super) fn pair_locks(env: &ActionEnv<'_>, state: &CompareState) -> io::Result<Vec<PairLock>> {
    let busy = |error: io::Error| {
        if error.kind() == io::ErrorKind::WouldBlock {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "Ein Lauf des Tresor-Jobs ist im Gang",
            )
        } else {
            error
        }
    };
    if let [a, b] = state.sides.as_slice() {
        let id = pair_lock_id(a.backend.as_ref(), &a.root, b.backend.as_ref(), &b.root);
        return Ok(vec![PairLock::acquire(&id).map_err(&busy)?]);
    }
    let Some(site) = state.sides.first().map(|side| side.site.as_str()) else {
        return Ok(Vec::new());
    };
    let jobs = crate::syncjobs::load_report()?.jobs;
    let mut locks = Vec::new();
    for job in jobs
        .iter()
        .filter(|job| job.is_vault() && (job.vault_site_a == site || job.vault_site_b == site))
    {
        // A job whose sides are unreachable cannot run, so it holds no lock.
        if let Ok(sides) = super::sync_job::prepare(env.store, env.resolve, job) {
            let id = pair_lock_id(
                sides.a.0.as_ref(),
                &sides.a.1,
                sides.b.0.as_ref(),
                &sides.b.1,
            );
            locks.push(PairLock::acquire(&id).map_err(&busy)?);
        }
    }
    Ok(locks)
}
