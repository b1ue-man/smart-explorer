//! `sync.options/jobs/validate/save/delete/setEnabled`: the desktop job files
//! (`syncjobs`) edited through the desktop `JobEditor` validation.
use serde_json::{json, Value};

use super::args::{bool_arg, invalid, io_error, str_arg};
use super::job_json::{apply_draft, field_for_message, job_json, FieldErrors};
use super::job_json::{CALENDAR_DAILY, CALENDAR_MONTHLY, CALENDAR_WEEKLY};
use super::{sync_conflicts, sync_run};
use crate::bisync::{CompareMode, ConflictMode, DeletePolicy, Direction, VersioningScheme};
use crate::mobile::{ApiError, Runtime};
use crate::syncjobs::editor::JobEditor;
use crate::syncjobs::{SyncJob, Trigger};

pub(super) fn load_jobs() -> Result<Vec<SyncJob>, ApiError> {
    crate::syncjobs::load().map_err(|error| io_error("Sync-Jobs laden", error))
}

pub(super) fn find_job(id: &str) -> Result<SyncJob, ApiError> {
    load_jobs()?
        .into_iter()
        .find(|job| job.id == id)
        .ok_or_else(|| ApiError::new("not_found", "Sync-Job nicht gefunden."))
}

pub(super) fn notify_jobs(rt: &Runtime) {
    rt.emit(json!({ "type": "jobs" }));
}

fn one_job(job: &SyncJob) -> Value {
    let results = crate::syncjobs::load_results();
    job_json(
        job,
        results.get(&job.id),
        sync_run::running_task(&job.id).as_deref(),
    )
}

pub(super) fn options() -> Result<Value, ApiError> {
    fn entry(value: &str, label: &str) -> Value {
        json!({ "value": value, "label": label })
    }
    let directions = [Direction::AtoB, Direction::BtoA, Direction::Both];
    let deletes = [
        DeletePolicy::Propagate,
        DeletePolicy::Mirror,
        DeletePolicy::NoDelete,
    ];
    let compares = [
        CompareMode::MtimeSize,
        CompareMode::SizeOnly,
        CompareMode::Checksum,
    ];
    // Device-arrival triggers never fire on Android (no removable-drive events).
    let triggers = Trigger::ALL
        .iter()
        .filter(|trigger| **trigger != Trigger::OnConnect);
    let mut defaults = SyncJob::new(String::new(), String::new(), String::new());
    defaults.id = String::new();
    Ok(json!({
        "directions": directions.iter().map(|d| entry(d.as_str(), d.label())).collect::<Vec<_>>(),
        "conflicts": ConflictMode::ALL.iter().map(|c| entry(c.as_str(), c.label())).collect::<Vec<_>>(),
        "deletePolicies": deletes.iter().map(|d| entry(d.as_str(), d.label())).collect::<Vec<_>>(),
        "compares": compares.iter().map(|c| entry(c.as_str(), c.label())).collect::<Vec<_>>(),
        "versionings": VersioningScheme::ALL.iter().map(|v| entry(v.as_str(), v.label())).collect::<Vec<_>>(),
        "triggers": triggers.map(|t| entry(t.as_str(), t.label())).collect::<Vec<_>>(),
        "calendarKinds": [
            entry(CALENDAR_DAILY, "Täglich"),
            entry(CALENDAR_WEEKLY, "Wöchentlich"),
            entry(CALENDAR_MONTHLY, "Monatlich"),
        ],
        "defaults": job_json(&defaults, None, None),
    }))
}

pub(super) fn jobs() -> Result<Value, ApiError> {
    let jobs = load_jobs()?;
    let results = crate::syncjobs::load_results();
    let list: Vec<Value> = jobs
        .iter()
        .map(|job| {
            job_json(
                job,
                results.get(&job.id),
                sync_run::running_task(&job.id).as_deref(),
            )
        })
        .collect();
    Ok(Value::Array(list))
}

struct Checked {
    job: Option<SyncJob>,
    errors: FieldErrors,
}

/// Desktop validation of a draft; `job` is set only when it is valid.
fn check_draft(args: &Value) -> Result<Checked, ApiError> {
    let draft = args
        .get("job")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("Parameter „job“ fehlt."))?;
    let id = draft.get("id").and_then(Value::as_str).unwrap_or("").trim();
    let existing = if id.is_empty() {
        None
    } else {
        Some(find_job(id)?)
    };
    let mut editor = match &existing {
        Some(job) => JobEditor::from_job(job),
        None => JobEditor::blank(String::new(), String::new()),
    };
    let mut errors = apply_draft(&mut editor, draft);
    for (field, location) in [("source", &editor.source), ("target", &editor.target)] {
        if location.trim().is_empty() {
            errors.insert(field.to_string(), "Bitte einen Ort wählen.".to_string());
        } else if Runtime::is_app_internal(location) {
            errors.insert(
                field.to_string(),
                "Orte in ZIP-Archiven oder im Papierkorb können nicht synchronisiert werden."
                    .to_string(),
            );
        }
    }
    if !errors.is_empty() {
        return Ok(Checked { job: None, errors });
    }
    match editor.build_sync_job(existing.as_ref()) {
        Ok(job) => Ok(Checked {
            job: Some(job),
            errors,
        }),
        Err(message) => {
            errors.insert(field_for_message(&message).to_string(), message);
            Ok(Checked { job: None, errors })
        }
    }
}

pub(super) fn validate(args: &Value) -> Result<Value, ApiError> {
    let checked = check_draft(args)?;
    Ok(json!({ "errors": checked.errors }))
}

pub(super) fn save(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let checked = check_draft(args)?;
    let job = match checked.job {
        Some(job) => job,
        None => {
            let message = checked
                .errors
                .values()
                .next()
                .cloned()
                .unwrap_or_else(|| "Ungültiges Sync-Setup.".to_string());
            return Err(invalid(message));
        }
    };
    crate::syncjobs::upsert(&job).map_err(|error| io_error("Sync-Job speichern", error))?;
    notify_jobs(rt);
    Ok(one_job(&job))
}

pub(super) fn delete(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let id = str_arg(args, "id")?;
    if sync_run::running_task(id).is_some() {
        return Err(ApiError::new(
            "busy",
            "Der Job läuft gerade – bitte warten oder abbrechen.",
        ));
    }
    crate::syncjobs::remove(id).map_err(|error| io_error("Sync-Job löschen", error))?;
    sync_conflicts::forget(id);
    notify_jobs(rt);
    Ok(json!({}))
}

pub(super) fn set_enabled(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let id = str_arg(args, "id")?;
    let enabled = bool_arg(args, "enabled")?;
    let mut job = find_job(id)?;
    job.enabled = enabled;
    crate::syncjobs::upsert(&job).map_err(|error| io_error("Sync-Job speichern", error))?;
    notify_jobs(rt);
    Ok(one_job(&job))
}
