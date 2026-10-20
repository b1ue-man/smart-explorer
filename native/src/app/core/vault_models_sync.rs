//! Models of the vault sync jobs in the desktop windows (U29-9): the fixed
//! and the selectable fields of the editor „Tresor-Sync“, the draft of the
//! dialog „Synchronisieren…“ with its second location, and the routing of a
//! row of the job window (run, compare, edit) for vault jobs. No `egui`, no
//! input/output.
#![allow(dead_code)] // some accessors are the acceptance signals (tests) and the surfaces of later packages
use crate::bisync::{CompareMode, DeletePolicy, Direction};
use crate::syncjobs::{SyncJob, Trigger};

use super::vault_models::VaultRow;

/// A selectable field of the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FieldId {
    Name,
    Direction,
    /// Only for a one-sided job; both ways always carry deletions over.
    DeletePolicy,
    Enabled,
    Trigger,
    IntervalMin,
    CalTime,
    CalWeekdays,
    CalMonthday,
    CatchUp,
    RtDebounce,
    RtMaxLatency,
    RtPoll,
    ConnectMatch,
    VerifyInterval,
    VerifyTarget,
    ActiveWindow,
}

/// What the editor shows of a job: the fixed values (label and value, grey,
/// with the hover „Bei Tresor-Jobs fest.“) and the fields to change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditorModel {
    pub(crate) fixed: Vec<(String, String)>,
    pub(crate) editable: Vec<FieldId>,
}

/// The hover text of every fixed field.
pub(crate) const FIXED_HOVER: &str = "Bei Tresor-Jobs fest.";

fn compare_text(mode: CompareMode) -> &'static str {
    match mode {
        CompareMode::Checksum => "Prüfsumme",
        _ => "Größe und Zeit",
    }
}

/// The fixed and the selectable fields of the job `job` (C5.6; the fixed
/// values the vault layer enforces in `SyncJob::validate`).
pub(crate) fn sync_editor_model(job: &SyncJob) -> EditorModel {
    editor_model_for(job.direction, job.compare)
}

/// The same for the values of a draft of the editor.
pub(crate) fn editor_model_for(direction: Direction, compare: CompareMode) -> EditorModel {
    let mut fixed = vec![(
        "Vergleich der Änderungen".to_string(),
        compare_text(compare).to_string(),
    )];
    if direction == Direction::Both {
        fixed.push(("Löschen".to_string(), "Übernehmen".to_string()));
    }
    for (label, value) in [
        ("Konflikte", "nie automatisch auflösen"),
        ("Sicherung", "–"),
        ("Filter", "keine"),
        ("Verschieben", "aus"),
        ("Versteckte Einträge", "eingeschlossen"),
        ("Ignoriermuster", "keine"),
        ("Versionen", "in den App-Daten"),
        ("Papierkorb", "nicht verwendet"),
        ("Kopien", "atomar"),
        ("Andere Dateisysteme", "nicht einbezogen"),
    ] {
        fixed.push((label.to_string(), value.to_string()));
    }
    let mut editable = vec![FieldId::Name, FieldId::Direction];
    if direction != Direction::Both {
        editable.push(FieldId::DeletePolicy);
    }
    editable.extend([
        FieldId::Enabled,
        FieldId::Trigger,
        FieldId::IntervalMin,
        FieldId::CalTime,
        FieldId::CalWeekdays,
        FieldId::CalMonthday,
        FieldId::CatchUp,
        FieldId::RtDebounce,
        FieldId::RtMaxLatency,
        FieldId::RtPoll,
        FieldId::ConnectMatch,
        FieldId::VerifyInterval,
        FieldId::VerifyTarget,
        FieldId::ActiveWindow,
    ]);
    EditorModel { fixed, editable }
}

/// The word of a direction in the vault windows (A and B are the two sites).
pub(crate) fn direction_label(direction: Direction) -> &'static str {
    match direction {
        Direction::Both => "Beidseitig",
        Direction::AtoB => "Nur A → B",
        Direction::BtoA => "Nur B → A",
    }
}

/// The word of a deletion policy of a one-sided job.
pub(crate) fn delete_label(policy: DeletePolicy) -> &'static str {
    match policy {
        DeletePolicy::Propagate => "Übernehmen",
        DeletePolicy::Mirror => "Spiegeln",
        DeletePolicy::NoDelete => "Nichts löschen",
    }
}

/// The policies a one-sided job may choose.
pub(crate) const ONE_SIDED_DELETE: [DeletePolicy; 3] = [
    DeletePolicy::Propagate,
    DeletePolicy::Mirror,
    DeletePolicy::NoDelete,
];

/// Puts the draft values the direction fixes in place: both ways carry
/// deletions over (C5.6), and a move needs one direction and is fixed off.
pub(crate) fn normalize(direction: Direction, policy: DeletePolicy) -> DeletePolicy {
    if direction == Direction::Both {
        DeletePolicy::Propagate
    } else {
        policy
    }
}

/// Where the second side of a new job comes from (C5.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TargetChoice {
    /// A new copy se keeps (the default).
    Replica,
    /// An entered site of the same vault (its ID).
    Site(String),
    /// A new or empty folder at any se location.
    Folder,
}

/// The draft of the dialog „Synchronisieren…“.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CreateDraft {
    pub(crate) name: String,
    pub(crate) direction: Direction,
    pub(crate) delete_policy: DeletePolicy,
    pub(crate) target: TargetChoice,
    pub(crate) folder: String,
}

impl CreateDraft {
    pub(crate) fn new(origin_label: &str) -> Self {
        Self {
            name: default_job_name(origin_label),
            direction: Direction::Both,
            delete_policy: DeletePolicy::Propagate,
            target: TargetChoice::Replica,
            folder: String::new(),
        }
    }

    /// Why „Anlegen“ is not possible yet.
    pub(crate) fn issue(&self) -> Option<String> {
        match &self.target {
            TargetChoice::Site(site) if site.is_empty() => {
                Some("Bitte einen Tresorort wählen.".to_string())
            }
            TargetChoice::Folder if self.folder.trim().is_empty() => {
                Some("Bitte einen Ordner angeben.".to_string())
            }
            _ => None,
        }
    }
}

/// The name of a new job.
pub(crate) fn default_job_name(origin_label: &str) -> String {
    format!("Tresor {origin_label}")
}

/// The entered sites of the same vault as `origin`, other than `origin`.
pub(crate) fn same_vault_sites<'a>(rows: &'a [VaultRow], origin: &VaultRow) -> Vec<&'a VaultRow> {
    rows.iter()
        .filter(|row| {
            row.site != origin.site
                && !origin.record.vault_id.is_empty()
                && row.record.vault_id == origin.record.vault_id
        })
        .collect()
}

/// Which runner a job of the job window goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunKind {
    /// The general runners (`sync_core`, `sync_manual_run`).
    General,
    /// `vault::run_vault_job` (rules R1–R11, no automatic resolution).
    Vault,
}

/// What „Vergleichen“ of a job shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CompareKind {
    /// The preview of the general sync.
    Preview,
    /// The comparison window „Tresor vergleichen“.
    VaultCompare,
}

/// A row of the job window as far as vault jobs change it (U29-9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct JobRowModel {
    /// The name with the addition „(Tresor)“ for a vault job.
    pub(crate) title: String,
    pub(crate) run: RunKind,
    pub(crate) compare: CompareKind,
    /// The editor of the vault job (pages „Tresor“ and „Zeitplan“).
    pub(crate) vault_editor: bool,
    /// The version history of the general job does not exist.
    pub(crate) has_versions: bool,
}

pub(crate) fn job_row(job: &SyncJob) -> JobRowModel {
    let name = if job.name.trim().is_empty() {
        "(ohne Name)"
    } else {
        job.name.as_str()
    };
    if job.is_vault() {
        JobRowModel {
            title: format!("{name} (Tresor)"),
            run: RunKind::Vault,
            compare: CompareKind::VaultCompare,
            vault_editor: true,
            has_versions: false,
        }
    } else {
        JobRowModel {
            title: name.to_string(),
            run: RunKind::General,
            compare: CompareKind::Preview,
            vault_editor: false,
            has_versions: true,
        }
    }
}

/// A broken vault job as the list shows it.
pub(crate) fn broken_job_text(id: &str, reason: &str) -> String {
    format!("{id}: fehlerhaft: {reason}")
}

/// The broken files of `vault-jobs/` in a load report as lines (the report
/// also names broken files of `jobs/`, which the general list owns).
pub(crate) fn broken_vault_job_lines(report: &crate::syncjobs::JobLoadReport) -> Vec<String> {
    report
        .broken
        .iter()
        .filter(|broken| {
            broken
                .path
                .parent()
                .and_then(|parent| parent.file_name())
                .is_some_and(|name| name == crate::syncjobs::VAULT_JOBS_DIR)
        })
        .map(|broken| broken_job_text(&broken.id, &broken.error))
        .collect()
}

/// The schedule of a job in one line for the properties page „Sync“.
pub(crate) fn schedule_text(job: &SyncJob) -> String {
    match job.trigger {
        Trigger::Manual => "manuell".to_string(),
        Trigger::Interval => format!("alle {} min", job.interval_min),
        Trigger::RealTime => "Echtzeit".to_string(),
        Trigger::Calendar => "Zeitplan".to_string(),
        Trigger::OnStartup => "beim Start".to_string(),
        Trigger::OnConnect => "bei Geräteanschluss".to_string(),
    }
}
