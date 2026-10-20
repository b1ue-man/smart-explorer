//! Text and JSON of `se sync list/show`. JSON keys are the keys of the saved
//! job file, so `se sync show --json` and `se sync edit --<key>` name every
//! setting the same way; `state` is the runtime state the job lists show.
use serde_json::{json, Value};

use crate::syncjobs::editor::min_to_hm;
use crate::syncjobs::{JobState, SyncJob, Trigger};

use super::options::WEEKDAYS;

pub(super) fn job_value(job: &SyncJob, state: Option<&JobState>, now: i64) -> Value {
    // Two halves: one `json!` with every key exceeds the macro recursion limit.
    let mut value = json!({
        "id": job.id,
        "name": job.name,
        "source": job.source,
        "target": job.target,
        "enabled": job.enabled,
        "schedule": schedule_text(job),
        "status": state.map(|state| status_text(state, now)),
        "direction": job.direction.as_str(),
        "conflict": job.conflict.as_str(),
        "delete_policy": job.delete_policy.as_str(),
        "move_files": job.move_files,
        "compare": job.compare.as_str(),
        "modify_window_sec": job.modify_window_sec,
        "versioning_scheme": job.versioning_scheme.as_str(),
        "retain_days": job.retain_days,
        "retain_count": job.retain_count,
        "versions_location": job.versions_location.as_str(),
        "use_recycle_bin": job.use_recycle_bin,
        "max_delete": job.max_delete,
        "max_delete_pct": job.max_delete_pct,
        "max_delete_min": job.max_delete_min,
        "trigger": job.trigger.as_str(),
        "interval_min": job.interval_min,
        "cal_time": min_to_hm(job.cal_time_min),
        "cal_weekdays": weekday_names(job.cal_weekdays),
        "cal_monthday": job.cal_monthday,
        "rt_debounce_secs": job.rt_debounce_secs,
        "rt_max_latency_secs": job.rt_max_latency_secs,
        "rt_poll_secs": job.rt_poll_secs,
        "verify_interval_secs": job.verify_interval_secs,
        "verify_target_secs": job.verify_target_secs,
    });
    let rest = json!({
        "connect_match": job.connect_match,
        "active_from": min_to_hm(job.active_from_min),
        "active_to": min_to_hm(job.active_to_min),
        "catch_up": job.catch_up,
        "include_hidden": job.include_hidden,
        "ignore": job.ignore,
        "filter_min_size_kb": job.filter_min_size_kb,
        "filter_max_size_kb": job.filter_max_size_kb,
        "filter_max_age_days": job.filter_max_age_days,
        "filter_min_age_days": job.filter_min_age_days,
        "cross_mounts": job.cross_mounts,
        "bwlimit_kbps": job.bwlimit_kbps,
        "max_transfers": job.max_transfers,
        "atomic_copy": job.atomic_copy,
        "verify": job.verify,
        "retries": job.retries,
        "retry_delay_secs": job.retry_delay_secs,
        "run_before": job.run_before,
        "run_after": job.run_after,
        "run_cleanup": job.run_cleanup,
        "log_verbose": crate::bisync::job_log_verbose(&job.id),
        "last_run": job.last_run,
        "config_version": job.config_version,
        "state": state.and_then(|state| serde_json::to_value(state).ok()),
    });
    if let (Value::Object(map), Value::Object(rest)) = (&mut value, rest) {
        map.extend(rest);
    }
    // Only vault jobs carry the kind, so the JSON of every other job stays
    // as it was.
    if let (true, Value::Object(map)) = (job.is_vault(), &mut value) {
        map.insert("kind".into(), json!("vault"));
    }
    value
}

/// Whether a job file that cannot be read lies in `vault-jobs/`.
pub(super) fn is_vault_broken(broken: &crate::syncjobs::BrokenJob) -> bool {
    broken.path.starts_with(crate::syncjobs::vault_jobs_dir())
}

/// A job that cannot be read, as `se sync list --json` lists it; a vault
/// job says so and is marked `broken`.
pub(super) fn broken_value(broken: &crate::syncjobs::BrokenJob) -> Value {
    if is_vault_broken(broken) {
        json!({
            "id": broken.id,
            "kind": "vault",
            "broken": true,
            "broken_config": broken.error,
        })
    } else {
        json!({ "id": broken.id, "broken_config": broken.error })
    }
}

/// One line of `se sync list`.
pub(super) fn list_line(job: &SyncJob, state: Option<&JobState>, now: i64) -> String {
    clean(&format!(
        "{}\t{}\t{}\t{}\t{} -> {}\t{}{}",
        job.id,
        job.name,
        if job.enabled { "on" } else { "off" },
        schedule_text(job),
        job.source,
        job.target,
        state.map_or_else(|| "never run".to_string(), |state| status_text(state, now)),
        if job.is_vault() { "\tvault" } else { "" },
    ))
}

/// `key<TAB>value` lines of `se sync show`, sorted by key (the keys of the
/// job file and of `--json`).
pub(super) fn show_lines(value: &Value) -> Vec<String> {
    let Some(fields) = value.as_object() else {
        return Vec::new();
    };
    fields
        .iter()
        .filter(|(key, _)| key.as_str() != "state")
        .map(|(key, field)| {
            let text = match field {
                Value::String(text) => text.clone(),
                Value::Array(items) => items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .map_or_else(|| item.to_string(), str::to_string)
                    })
                    .collect::<Vec<_>>()
                    .join(","),
                Value::Null => "-".to_string(),
                other => other.to_string(),
            };
            clean(&format!("{key}\t{text}"))
        })
        .collect()
}

/// The trigger description of the job lists (desktop `menus_sync_jobs`,
/// Android `schedule`), in English.
pub(super) fn schedule_text(job: &SyncJob) -> String {
    match job.trigger {
        Trigger::Manual => "manual".to_string(),
        Trigger::Interval => format!("every {} min", job.interval_min),
        Trigger::Calendar => {
            let time = min_to_hm(job.cal_time_min);
            if job.cal_monthday != 0 {
                format!("monthly on day {} at {time}", job.cal_monthday)
            } else if job.cal_weekdays == 0 {
                format!("daily {time}")
            } else {
                format!("{} {time}", weekday_names(job.cal_weekdays).join(","))
            }
        }
        Trigger::RealTime => format!("realtime (+{}s)", job.rt_debounce_secs),
        Trigger::OnStartup => "at startup".to_string(),
        Trigger::OnConnect if job.connect_match.is_empty() => "on any removable drive".to_string(),
        Trigger::OnConnect => format!("on drive {:?}", job.connect_match),
    }
}

fn weekday_names(mask: u8) -> Vec<&'static str> {
    WEEKDAYS
        .iter()
        .enumerate()
        .filter(|(day, _)| (mask >> day) & 1 == 1)
        .map(|(_, name)| *name)
        .collect()
}

fn status_text(state: &JobState, now: i64) -> String {
    if state.running_now(now).is_some() {
        "running".to_string()
    } else if let Some(blocked) = &state.blocked {
        format!("safety stop: {}", blocked.detail)
    } else if state.consecutive_failures > 0 {
        let error = state
            .last_error
            .as_ref()
            .map(|error| format!(": {}", error.message))
            .unwrap_or_default();
        format!("failing ({} in a row){error}", state.consecutive_failures)
    } else if let Some(at) = state.last_success {
        format!("ok (last success {at})")
    } else {
        "never run".to_string()
    }
}

fn clean(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luecken_task_sync_cli_json_names_every_setting_like_the_job_file() {
        let job = SyncJob::new("Docs".into(), "/a".into(), "/b".into());
        let value = job_value(&job, None, 0);
        // Every editable field of the desktop editor (`JobEditor`) has its key.
        for key in [
            "name",
            "source",
            "target",
            "direction",
            "conflict",
            "delete_policy",
            "move_files",
            "compare",
            "modify_window_sec",
            "versioning_scheme",
            "retain_days",
            "retain_count",
            "versions_location",
            "use_recycle_bin",
            "max_delete",
            "max_delete_pct",
            "max_delete_min",
            "trigger",
            "interval_min",
            "cal_time",
            "cal_weekdays",
            "cal_monthday",
            "rt_debounce_secs",
            "rt_max_latency_secs",
            "rt_poll_secs",
            "verify_interval_secs",
            "verify_target_secs",
            "connect_match",
            "active_from",
            "active_to",
            "catch_up",
            "include_hidden",
            "ignore",
            "filter_min_size_kb",
            "filter_max_size_kb",
            "filter_max_age_days",
            "filter_min_age_days",
            "cross_mounts",
            "bwlimit_kbps",
            "max_transfers",
            "atomic_copy",
            "verify",
            "retries",
            "retry_delay_secs",
            "run_before",
            "run_after",
            "run_cleanup",
            "enabled",
            "log_verbose",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
        assert_eq!(value["cal_time"], "09:00");
        assert_eq!(value["direction"], "both");
        assert!(show_lines(&value).iter().any(|line| line == "name\tDocs"));
    }

    #[test]
    fn luecken_task_sync_cli_schedule_text_follows_the_trigger() {
        let mut job = SyncJob::new("S".into(), "/a".into(), "/b".into());
        assert_eq!(schedule_text(&job), "manual");
        job.trigger = Trigger::Interval;
        job.interval_min = 15;
        assert_eq!(schedule_text(&job), "every 15 min");
        job.trigger = Trigger::Calendar;
        job.cal_weekdays = 1 | 4;
        assert_eq!(schedule_text(&job), "mon,wed 09:00");
        job.cal_monthday = 3;
        assert_eq!(schedule_text(&job), "monthly on day 3 at 09:00");
        job.trigger = Trigger::OnConnect;
        assert_eq!(schedule_text(&job), "on any removable drive");
    }

    #[test]
    fn vault_task_cli_sync_list_marks_vault_jobs() {
        const A: &str = "0123456789abcdef0123456789abcdef";
        const B: &str = "fedcba9876543210fedcba9876543210";
        // The JSON and the rows of general jobs stay as they were.
        let general = SyncJob::new("Docs".into(), "/a".into(), "/b".into());
        assert!(job_value(&general, None, 0).get("kind").is_none());
        assert!(!list_line(&general, None, 0).ends_with("\tvault"));
        // A vault job says so, in the row and in the JSON.
        let vault = SyncJob::new_vault("Tresor".into(), A, B);
        assert_eq!(job_value(&vault, None, 0)["kind"], "vault");
        assert!(list_line(&vault, None, 0).ends_with("\tvault"));
        // A vault job that cannot be read is marked broken, with its reason;
        // a general one is listed as before.
        let broken_vault = crate::syncjobs::BrokenJob {
            id: "kaputt".into(),
            path: crate::syncjobs::vault_jobs_dir().join("kaputt.conf"),
            error: "Zeile 3".into(),
        };
        let value = broken_value(&broken_vault);
        assert_eq!(value["broken"], true);
        assert_eq!(value["kind"], "vault");
        assert_eq!(value["broken_config"], "Zeile 3");
        let broken_general = crate::syncjobs::BrokenJob {
            id: "alt".into(),
            path: crate::syncjobs::jobs_dir().join("alt.conf"),
            error: "Zeile 1".into(),
        };
        let value = broken_value(&broken_general);
        assert!(value.get("broken").is_none() && value.get("kind").is_none());
        assert_eq!(value["broken_config"], "Zeile 1");
    }
}
