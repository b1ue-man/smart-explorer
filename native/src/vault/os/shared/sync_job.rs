//! Running a vault sync job (vault plan C5, VLT-M7): the general bisync
//! engine over the encrypted storage of two vault sites (C5.1: no key, so
//! the job runs while the vault is locked), with the rules R1–R11 installed
//! on the running thread (`bisync::plan_rules::enter`) and always as a full
//! scan (R4a).
//!
//! Before planning both sides are checked (R7): `vault.cryptomator` with the
//! job's vault ID and `d/` on each side, else the run does not start and
//! nothing changes anywhere; only a copy still being seeded may be empty.
//! Different vault IDs abort the run (R1). After a run without error,
//! conflict, omission or deferral the seeding copy is complete (C5.9).
//!
//! Every runner of jobs (background service, desktop, Android, CLI) runs a
//! vault job through `run_vault_job`; the general engine options of a vault
//! job are refused (`SyncJob::checked_opts`), so a runner that does not know
//! vault jobs fails safely.
use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::bisync::plan_rules::{self, Action, PlanRules};
use crate::bisync::{ApplySink, Outcome, PairSide, Preview, RunSettings, ScanDepth, StateOwner};
use crate::cryptomator::DATA_DIR;
use crate::syncjobs::SyncJob;
use crate::vault::core::path_map::storage_path;
use crate::vault::core::registry_types::SiteRecord;
use crate::vault::core::sync_rules::{order_allows, VaultRules};
use crate::vfs::{Backend, BackendHandle};

use super::open::{resolve_store_in, ResolveLocation};
use super::registry_store::RegistryStore;
use super::replica::vault_id_at;

/// One run of a vault job.
pub(crate) struct VaultRunRequest<'a> {
    pub(crate) job: &'a SyncJob,
    pub(crate) dry_run: bool,
    pub(crate) cancel: &'a AtomicBool,
    /// Confirmations and lock wait of the caller; owner and depth are set
    /// here (the job's own state, always a full scan).
    pub(crate) settings: RunSettings,
    pub(crate) observer: Option<&'a dyn ApplySink>,
}

/// The result of a vault job run.
#[derive(Default)]
pub(crate) struct VaultRunOutcome {
    pub(crate) outcome: Outcome,
    /// R7/R1: why the run did not start; nothing changed anywhere.
    pub(crate) unavailable: Option<String>,
    /// The seeding copy (its site ID) finished its first complete run.
    pub(crate) seeded: Option<String>,
    /// How many plans the vault rules adjusted (every planned run: ≥ 1).
    pub(crate) planned_with_rules: usize,
}

/// Both checked sides of a vault job.
pub(super) struct Sides {
    pub(super) a: (BackendHandle, String),
    pub(super) b: (BackendHandle, String),
    pub(super) records: [SiteRecord; 2],
    pub(super) seeding: Option<PairSide>,
}

fn unavailable(stage: &str, reason: String) -> VaultRunOutcome {
    VaultRunOutcome {
        outcome: Outcome {
            errors: vec![(stage.to_string(), reason.clone())],
            stats: crate::bisync::BisyncStats {
                errors: 1,
                ..Default::default()
            },
            ..Outcome::default()
        },
        unavailable: Some(reason),
        ..VaultRunOutcome::default()
    }
}

/// What one side holds (R7).
enum Observed {
    Vault {
        jti: String,
    },
    /// A seeding copy without `vault.cryptomator` yet.
    Empty,
}

fn observe(inner: &dyn Backend, root: &str, record: &SiteRecord) -> Result<Observed, String> {
    let label = record.display_label();
    let Some(jti) = vault_id_at(inner, root).map_err(|error| format!("{label}: {error}"))? else {
        return if record.seeding {
            Ok(Observed::Empty)
        } else {
            Err(format!(
                "{label}: kein Tresor am Speicherort (vault.cryptomator fehlt)"
            ))
        };
    };
    if !record.vault_id.is_empty() && record.vault_id != jti {
        return Err(format!(
            "{label}: am Speicherort liegt ein anderer Tresor als eingetragen"
        ));
    }
    match inner.stat(&storage_path(root, DATA_DIR)) {
        Ok(meta) if meta.is_dir => Ok(Observed::Vault { jti }),
        Ok(_) => Err(format!("{label}: d/ ist kein Ordner")),
        // A copy still being seeded may lack it until its first run ends.
        Err(error) if error.kind() == io::ErrorKind::NotFound && record.seeding => {
            Ok(Observed::Vault { jti })
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(format!("{label}: Tresor ohne d/ (unvollständig)"))
        }
        Err(error) => Err(format!("{label}: Speicherort nicht erreichbar: {error}")),
    }
}

/// Resolves and checks both sides (R7, R1, seeding).
pub(super) fn prepare(
    store: &RegistryStore,
    resolve: ResolveLocation<'_>,
    job: &SyncJob,
) -> Result<Sides, VaultRunOutcome> {
    let mut records = Vec::with_capacity(2);
    let mut storages = Vec::with_capacity(2);
    for site in [&job.vault_site_a, &job.vault_site_b] {
        let record = match store.get(site) {
            Ok(Some(record)) => record,
            Ok(None) => {
                return Err(unavailable(
                    "Tresor-Seite",
                    format!("Tresorort {site} ist nicht eingetragen"),
                ))
            }
            Err(error) => return Err(unavailable("Tresorliste", error.to_string())),
        };
        let (inner, root) = resolve_store_in(store, site, resolve).map_err(|error| {
            unavailable(
                "Tresor-Seite",
                format!("{}: nicht erreichbar: {error}", record.display_label()),
            )
        })?;
        storages.push((crate::vfs::sync_backend(inner), root));
        records.push(record);
    }
    let mut observed = Vec::with_capacity(2);
    for ((inner, root), record) in storages.iter().zip(&records) {
        observed.push(
            observe(inner.as_ref(), root, record)
                .map_err(|reason| unavailable("Tresor-Seite", reason))?,
        );
    }
    if let [Observed::Vault { jti: a }, Observed::Vault { jti: b }] = observed.as_slice() {
        if a != b {
            return Err(unavailable(
                "Tresor-ID",
                "Lauf abgebrochen: beide Seiten enthalten verschiedene Tresore (R1)".into(),
            ));
        }
    }
    if !records[0].vault_id.is_empty()
        && !records[1].vault_id.is_empty()
        && records[0].vault_id != records[1].vault_id
    {
        return Err(unavailable(
            "Tresor-ID",
            "Lauf abgebrochen: die Tresororte gehören zu verschiedenen Tresoren (R1)".into(),
        ));
    }
    let seeding = match (records[0].seeding, records[1].seeding) {
        (false, false) => None,
        (true, false) => Some(PairSide::A),
        (false, true) => Some(PairSide::B),
        (true, true) => {
            return Err(unavailable(
                "Tresor-Seite",
                "Beide Seiten sind Kopien im Aufbau; eine Seite muss der Tresor sein".into(),
            ))
        }
    };
    // A copy still being seeded never is the only source (an empty source
    // of a one-way job would delete the vault on the other side).
    let source = match job.direction {
        crate::bisync::Direction::AtoB => Some(PairSide::A),
        crate::bisync::Direction::BtoA => Some(PairSide::B),
        crate::bisync::Direction::Both => None,
    };
    if seeding.is_some() && seeding == source {
        return Err(unavailable(
            "Tresor-Seite",
            "Eine Kopie im Aufbau kann nicht die Quelle eines einseitigen Tresor-Jobs sein".into(),
        ));
    }
    let mut storages = storages.into_iter();
    let (Some(a), Some(b)) = (storages.next(), storages.next()) else {
        return Err(unavailable("Tresor-Seite", "Seiten fehlen".into()));
    };
    let [record_a, record_b]: [SiteRecord; 2] = records
        .try_into()
        .map_err(|_| unavailable("Tresor-Seite", "Seiten fehlen".into()))?;
    Ok(Sides {
        a,
        b,
        records: [record_a, record_b],
        seeding,
    })
}

fn settings_for(job: &SyncJob, mut settings: RunSettings) -> RunSettings {
    settings.owner = StateOwner::Job(job.id.clone());
    // R4a: every run of a vault job is a full scan with the rule stage.
    settings.depth = ScanDepth::Full;
    settings
}

/// A run without anything left open: the seeding copy is complete (C5.9).
fn complete(outcome: &Outcome, cancel: &AtomicBool) -> bool {
    outcome.stats.errors == 0
        && outcome.errors.is_empty()
        && outcome.conflicts.is_empty()
        && outcome.deferred.is_empty()
        && outcome.omissions.is_empty()
        && outcome.blocked.is_none()
        && outcome.stopped.is_none()
        && !outcome.busy
        && !outcome.canceled
        && !cancel.load(std::sync::atomic::Ordering::Acquire)
}

/// Runs a vault job with the vault list and resolution of this
/// installation.
pub(crate) fn run_vault_job(request: VaultRunRequest<'_>) -> VaultRunOutcome {
    let resolve = |location: &str| crate::connect::resolve_endpoint(location);
    run_vault_job_in(&RegistryStore::open_default(), &resolve, request)
}

/// `run_vault_job` against a given vault list and resolver.
pub(crate) fn run_vault_job_in(
    store: &RegistryStore,
    resolve: ResolveLocation<'_>,
    request: VaultRunRequest<'_>,
) -> VaultRunOutcome {
    let job = request.job;
    let opts = match job.checked_vault_opts(request.dry_run) {
        Ok(opts) => opts,
        Err(error) => return unavailable("Tresor-Job", error),
    };
    let sides = match prepare(store, resolve, job) {
        Ok(sides) => sides,
        Err(outcome) => return outcome,
    };
    let rules = Arc::new(VaultRules::new(sides.seeding));
    let installed: Arc<dyn PlanRules> = rules.clone();
    let _rules = plan_rules::enter(installed);
    let globs = crate::bisync::empty_globset();
    let filter = crate::bisync::WalkFilter::basic(true, &globs);
    let ((a, root_a), (b, root_b)) = (&sides.a, &sides.b);
    let mut outcome = crate::bisync::run_with(crate::bisync::RunRequest {
        a: a.as_ref(),
        root_a,
        b: b.as_ref(),
        root_b,
        opts,
        filter: &filter,
        cancel: request.cancel,
        settings: settings_for(job, request.settings),
        observer: request.observer,
    });
    let mut seeded = None;
    if let Some(side) = sides.seeding {
        if !request.dry_run && complete(&outcome, request.cancel) {
            let index = usize::from(side == PairSide::B);
            let mut record = sides.records[index].clone();
            record.seeding = false;
            match store.save(&record) {
                Ok(()) => seeded = Some(record.id),
                // The copy is complete; the next run repeats the check.
                Err(error) => {
                    outcome.errors.push(("Tresorliste".into(), error));
                    outcome.stats.errors = outcome.stats.errors.saturating_add(1);
                }
            }
        }
    }
    VaultRunOutcome {
        outcome,
        unavailable: None,
        seeded,
        planned_with_rules: rules.adjusted(),
    }
}

/// A comparison of a vault job (the vault comparison mode, C6): the
/// engine's preview with the vault rules, plus the actions the rules hold
/// back with their reasons.
pub(crate) struct VaultPreview {
    pub(crate) preview: Preview,
    pub(crate) deferred: Vec<(Action, String)>,
    pub(crate) unavailable: Option<String>,
}

/// Previews a vault job against a given vault list and resolver.
pub(crate) fn preview_vault_job_in(
    store: &RegistryStore,
    resolve: ResolveLocation<'_>,
    job: &SyncJob,
    cancel: &AtomicBool,
) -> VaultPreview {
    let failed = |reason: String| VaultPreview {
        preview: Preview {
            error: Some(reason.clone()),
            ..Preview::default()
        },
        deferred: Vec::new(),
        unavailable: Some(reason),
    };
    let opts = match job.checked_vault_opts(true) {
        Ok(opts) => opts,
        Err(error) => return failed(error),
    };
    let sides = match prepare(store, resolve, job) {
        Ok(sides) => sides,
        Err(outcome) => return failed(outcome.unavailable.unwrap_or_default()),
    };
    let rules = Arc::new(VaultRules::new(sides.seeding));
    let installed: Arc<dyn PlanRules> = rules.clone();
    let _rules = plan_rules::enter(installed);
    let globs = crate::bisync::empty_globset();
    let filter = crate::bisync::WalkFilter::basic(true, &globs);
    let ((a, root_a), (b, root_b)) = (&sides.a, &sides.b);
    let preview = crate::bisync::preview_with(
        a.as_ref(),
        root_a,
        b.as_ref(),
        root_b,
        opts,
        cancel,
        &filter,
        settings_for(job, RunSettings::default()),
    );
    let deferred = preview
        .actions
        .iter()
        .filter_map(|action| {
            rules
                .rule_reason(action)
                .map(|reason| (action.clone(), reason.to_string()))
        })
        .collect();
    VaultPreview {
        preview,
        deferred,
        unavailable: None,
    }
}

/// Applies one action of a vault preview (C6.4 „Jetzt übertragen“) only in
/// the order of R4: an action the rules hold back, or one that must wait
/// for an action of phase 1 still in `pending` (the preview's actions not
/// yet applied), is refused. The caller holds the vault job's storage
/// sides of the preview.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_vault_preview_action(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    preview: &VaultPreview,
    pending: &[Action],
    action: &Action,
    job: &SyncJob,
    cancel: &AtomicBool,
) -> io::Result<crate::bisync::BisyncStats> {
    if let Some((_, reason)) = preview.deferred.iter().find(|(held, _)| held == action) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            reason.clone(),
        ));
    }
    order_allows(pending, action)
        .map_err(|reason| io::Error::new(io::ErrorKind::WouldBlock, reason))?;
    let opts = job
        .checked_vault_opts(false)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    crate::bisync::apply_preview_action(
        a,
        root_a,
        b,
        root_b,
        &preview.preview,
        action,
        opts,
        cancel,
    )
}

#[cfg(test)]
#[path = "sync_job_more_tests.rs"]
mod more_tests;
#[cfg(test)]
#[path = "sync_job_test_support.rs"]
mod test_support;
#[cfg(test)]
#[path = "sync_job_tests.rs"]
mod tests;
