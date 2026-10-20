//! „Tresor prüfen“ (vault plan C6.2, C6.6, U26-7): the findings of the walk
//! over the encrypted storage of one site (`dir_map.rs`) become entries of
//! the group `structure` with the actions of C6.2. The check needs the key
//! of the site (without it: `vault_locked`) and works on one location; a
//! comparison adds the findings of both sides when it is asked for the group.
//! The fixes are in `health_fixes.rs`.
use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::cryptomator::DIR_FILE;
use crate::vault::core::compare_model::{CompareGroup, CompareItem, ItemKind, SideState};
use crate::vault::core::health_model::{needs_free_pair_lock, FindingKind, HealthFinding};
use crate::vault::core::sync_rules::content_folder;
use crate::vault::core::{vault_locked, VaultSession};

use super::compare::{item_id, order_and_number};
use super::compare_cache::{cache, site_key, CompareState, Dropper, Held, Payload, SideIo};
use super::compare_items::locate;
use super::dir_map::DirMap;
use super::open::{resolve_store_in, unlocked_session, ResolveLocation};
use super::registry_store::RegistryStore;

/// What a check of one site found.
pub(crate) struct CheckResult {
    pub(crate) site: String,
    pub(crate) label: String,
    pub(crate) revision: u64,
    pub(crate) items: Vec<CompareItem>,
}

const SYNC_NOTE: &str = "kann ein Zwischenstand eines laufenden Syncs sein";

/// Whether the two nodes of a `DirIdCollision` lie in one folder under one
/// name (then the second is a plain duplicate).
fn same_parent_and_name(map: &DirMap, finding: &HealthFinding) -> bool {
    let Some(first) = finding.detail.as_deref() else {
        return false;
    };
    let entry_of = |node_file: &str| {
        node_file
            .strip_suffix(&format!("/{DIR_FILE}"))
            .map(str::to_string)
    };
    let (Some(second), Some(first)) = (entry_of(&finding.rel), entry_of(first)) else {
        return false;
    };
    let name_of = |entry: &str| {
        let (content, stored) = entry.rsplit_once('/')?;
        map.entry(content, stored).map(|info| info.clear.clone())
    };
    content_folder(&second) == content_folder(&first)
        && name_of(&second).is_some()
        && name_of(&second) == name_of(&first)
}

/// The entry of one finding on side `side` of `sides`.
fn finding_held(
    key: &str,
    sides: &[SideIo],
    maps: &[DirMap],
    side: usize,
    finding: &HealthFinding,
    sync_note: bool,
) -> Held {
    let located = locate(&finding.rel, &maps[side..=side]);
    let kind = match finding.kind {
        FindingKind::DirIdCollision
        | FindingKind::EmptyDirFile
        | FindingKind::ObeseDirFile
        | FindingKind::MissingContentDir
        | FindingKind::MissingDirIdBackup
        | FindingKind::OrphanContentDir => ItemKind::Folder,
        _ => located.kind,
    };
    let state_of = |index: usize| match sides.get(index) {
        Some(other) if index == side => SideState {
            exists: true,
            size: None,
            mtime_ms: None,
            location: other.label.clone(),
        },
        Some(other) => SideState::absent(&other.label),
        None => SideState::absent(""),
    };
    let mut note = finding.kind.describe().to_string();
    if sync_note && needs_free_pair_lock(finding.kind) {
        note = format!("{note} – {SYNC_NOTE}");
    }
    let identity = format!("{}|{}|{side}", finding.rel, finding.kind.name());
    let item = CompareItem {
        id: item_id(key, "structure", &identity),
        group: CompareGroup::Structure,
        kind,
        path: located.path,
        a: state_of(0),
        b: state_of(1),
        actions: finding
            .kind
            .actions(same_parent_and_name(&maps[side], finding)),
        finding: Some(finding.kind.name().to_string()),
        note: Some(note),
    };
    Held {
        item,
        payload: Payload::Finding {
            side,
            finding: finding.clone(),
        },
        done: false,
    }
}

/// The findings of both sides of a job comparison as entries.
pub(super) fn job_findings(
    job: &str,
    sides: &[SideIo],
    maps: &[DirMap],
    _session: &VaultSession,
) -> Vec<Held> {
    let mut helds = Vec::new();
    for (side, map) in maps.iter().enumerate() {
        for finding in map.findings.iter().filter(|f| !f.kind.is_healthy()) {
            helds.push(finding_held(job, sides, maps, side, finding, true));
        }
    }
    helds
}

/// Checks the site `site` (ID, ID prefix or label): `vault_locked` without
/// its session. The result is kept for the actions on its entries
/// (`Resolve` with the site as the job).
pub(crate) fn check_site_in(
    store: &RegistryStore,
    resolve: ResolveLocation<'_>,
    site: &str,
    cancel: &AtomicBool,
) -> io::Result<CheckResult> {
    let record = store
        .resolve(site)
        .map_err(|message| io::Error::new(io::ErrorKind::NotFound, message))?;
    let session = unlocked_session(&record.id).ok_or_else(|| vault_locked(&record.id))?;
    let (backend, root) = resolve_store_in(store, &record.id, resolve).map_err(io::Error::other)?;
    let map = DirMap::build(backend.as_ref(), &root, &session, cancel)?;
    let io_side = SideIo {
        backend,
        root,
        site: record.id.clone(),
        label: record.display_label().to_string(),
    };
    let jobs_use = crate::syncjobs::load_report().is_ok_and(|report| {
        report.jobs.iter().any(|job| {
            job.is_vault() && (job.vault_site_a == record.id || job.vault_site_b == record.id)
        })
    });
    let key = site_key(&record.id);
    let sides = vec![io_side];
    let maps = vec![map];
    let mut helds: Vec<Held> = maps[0]
        .findings
        .iter()
        .filter(|finding| !finding.kind.is_healthy())
        .map(|finding| finding_held(&key, &sides, &maps, 0, finding, jobs_use))
        .collect();
    order_and_number(&mut helds);
    let items: Vec<CompareItem> = helds.iter().map(|held| held.item.clone()).collect();
    let revision = cache().next_revision();
    cache().put(CompareState {
        key,
        revision,
        site: record.id.clone(),
        session,
        job: None,
        unavailable: None,
        preview: None,
        sides,
        maps,
        items: Mutex::new(helds),
        listing: Arc::new(items.clone()),
        applied: Mutex::new(Vec::new()),
        listener: Arc::new(Dropper {
            site: record.id.clone(),
        }),
    });
    Ok(CheckResult {
        site: record.id.clone(),
        label: record.display_label().to_string(),
        revision,
        items,
    })
}

/// `check_site_in` with the vault list and the resolution of this
/// installation.
pub(crate) fn check_site(site: &str, cancel: &AtomicBool) -> io::Result<CheckResult> {
    let resolve = |location: &str| crate::connect::resolve_endpoint(location);
    check_site_in(&RegistryStore::open_default(), &resolve, site, cancel)
}
