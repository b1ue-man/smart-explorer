//! Acceptance signals of VLT-M10 (U29) on the vault sync jobs in the sync
//! windows: the fixed and selectable fields of the editor, the routing of a
//! job row to the vault runner and the draft of the dialog „Synchronisieren…“.
use super::vault_models_sync::{
    job_row, normalize, sync_editor_model, CompareKind, CreateDraft, FieldId, RunKind, TargetChoice,
};
use crate::bisync::{DeletePolicy, Direction};
use crate::syncjobs::SyncJob;

const SITE_A: &str = "0123456789abcdef0123456789abcdef";
const SITE_B: &str = "fedcba9876543210fedcba9876543210";

// ─── Sync jobs ────────────────────────────────────────────────────────────

#[test]
fn vault_task_gui_sync_editor_fixed_fields() {
    let job = SyncJob::new_vault("Tresor Privat".into(), SITE_A, SITE_B);
    let model = sync_editor_model(&job);
    let labels: Vec<&str> = model
        .fixed
        .iter()
        .map(|(label, _)| label.as_str())
        .collect();
    // The fixed values of C5.6 (U20, Abschnitt 3 Punkt 8) and the deletion
    // policy of a job in both directions.
    assert_eq!(
        labels,
        [
            "Vergleich der Änderungen",
            "Löschen",
            "Konflikte",
            "Sicherung",
            "Filter",
            "Verschieben",
            "Versteckte Einträge",
            "Ignoriermuster",
            "Versionen",
            "Papierkorb",
            "Kopien",
            "Andere Dateisysteme"
        ]
    );
    let value = |label: &str| {
        model
            .fixed
            .iter()
            .find(|(known, _)| known == label)
            .map(|(_, value)| value.as_str())
    };
    assert_eq!(value("Vergleich der Änderungen"), Some("Größe und Zeit"));
    assert_eq!(value("Löschen"), Some("Übernehmen"));
    assert_eq!(value("Konflikte"), Some("nie automatisch auflösen"));
    assert_eq!(value("Sicherung"), Some("–"));
    assert_eq!(value("Filter"), Some("keine"));
    // Selectable: name, direction, active, the schedule; the deletion
    // policy only for a one-sided job. No fixed field is among them.
    let editable = model.editable.clone();
    for field in [
        FieldId::Name,
        FieldId::Direction,
        FieldId::Enabled,
        FieldId::Trigger,
        FieldId::IntervalMin,
        FieldId::CalTime,
        FieldId::RtDebounce,
        FieldId::ActiveWindow,
    ] {
        assert!(editable.contains(&field), "{field:?}");
    }
    assert!(!editable.contains(&FieldId::DeletePolicy));
    let mut one_way = job.clone();
    one_way.direction = crate::bisync::Direction::AtoB;
    let one_way_model = sync_editor_model(&one_way);
    assert!(one_way_model.editable.contains(&FieldId::DeletePolicy));
    assert!(!one_way_model
        .fixed
        .iter()
        .any(|(label, _)| label == "Löschen"));
    // Every value the model calls fixed is one the vault checks enforce.
    let changes: [fn(&mut SyncJob); 5] = [
        |job| job.conflict = crate::bisync::ConflictMode::NewerWins,
        |job| job.include_hidden = false,
        |job| job.ignore = vec!["*.c9r".into()],
        |job| job.filter_max_size_kb = 5,
        |job| job.use_recycle_bin = true,
    ];
    for change in changes {
        let mut changed = job.clone();
        change(&mut changed);
        assert!(changed.validate().is_err());
    }
    // A job made with the checksum compare says so.
    let mut checksum = job.clone();
    checksum.compare = crate::bisync::CompareMode::Checksum;
    assert_eq!(
        sync_editor_model(&checksum).fixed[0].1,
        "Prüfsumme".to_string()
    );
}

#[test]
fn vault_task_gui_sync_run_uses_vault_runner() {
    let vault = SyncJob::new_vault("Privat".into(), SITE_A, SITE_B);
    let row = job_row(&vault);
    // „Jetzt ausführen“ of a vault row goes to `vault::run_vault_job` (the
    // desktop runner `sync_manual_run::run_vault`, tested there with
    // `vault_task_desktop_runs_vault_job_through_vault_sync`); its
    // comparison is the comparison window, its editor the vault editor.
    assert_eq!(row.run, RunKind::Vault);
    assert_eq!(row.compare, CompareKind::VaultCompare);
    assert!(row.vault_editor);
    assert!(!row.has_versions);
    assert_eq!(row.title, "Privat (Tresor)");
    // A general job keeps every way it had.
    let general = SyncJob::new("Fotos".into(), "/a".into(), "/b".into());
    let row = job_row(&general);
    assert_eq!(row.run, RunKind::General);
    assert_eq!(row.compare, CompareKind::Preview);
    assert!(!row.vault_editor);
    assert!(row.has_versions);
    assert_eq!(row.title, "Fotos");
    assert_eq!(
        job_row(&SyncJob::new(String::new(), "/a".into(), "/b".into())).title,
        "(ohne Name)"
    );
}

#[test]
fn vault_task_gui_sync_create_draft_needs_its_second_side() {
    let mut draft = CreateDraft::new("Privat");
    assert_eq!(draft.name, "Tresor Privat");
    assert_eq!(draft.target, TargetChoice::Replica);
    assert_eq!(draft.issue(), None, "the copy needs nothing more");
    draft.target = TargetChoice::Site(String::new());
    assert!(draft.issue().is_some());
    draft.target = TargetChoice::Site(SITE_B.into());
    assert_eq!(draft.issue(), None);
    draft.target = TargetChoice::Folder;
    assert!(draft.issue().is_some());
    draft.folder = "/mnt/zweit".into();
    assert_eq!(draft.issue(), None);
    // Both directions carry deletions over, whatever was chosen before.
    assert_eq!(
        normalize(Direction::Both, DeletePolicy::Mirror),
        DeletePolicy::Propagate
    );
    assert_eq!(
        normalize(Direction::AtoB, DeletePolicy::NoDelete),
        DeletePolicy::NoDelete
    );
}
