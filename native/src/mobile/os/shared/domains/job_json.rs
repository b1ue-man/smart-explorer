//! Sync-job JSON (api.md §4.7 `Job`) in both directions. Incoming drafts are
//! laid over the desktop `JobEditor`, so fields the phone does not show keep
//! their stored values and the desktop validation and texts apply unchanged.
use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use crate::bisync::{CompareMode, ConflictMode, DeletePolicy, Direction, VersioningScheme};
use crate::syncjobs::editor::{min_to_hm, JobEditor};
use crate::syncjobs::{JobResult, SyncJob, Trigger};

pub(super) const CALENDAR_DAILY: &str = "daily";
pub(super) const CALENDAR_WEEKLY: &str = "weekly";
pub(super) const CALENDAR_MONTHLY: &str = "monthly";

/// Field → German message, as returned by `sync.validate`.
pub(super) type FieldErrors = BTreeMap<String, String>;

pub(super) fn job_json(
    job: &SyncJob,
    result: Option<&JobResult>,
    running_task: Option<&str>,
) -> Value {
    json!({
        "id": job.id,
        "name": job.name,
        "source": job.source,
        "target": job.target,
        "direction": job.direction.as_str(),
        "conflict": job.conflict.as_str(),
        "deletePolicy": job.delete_policy.as_str(),
        "compare": job.compare.as_str(),
        "versioning": job.versioning_scheme.as_str(),
        "retainDays": job.retain_days,
        "trigger": job.trigger.as_str(),
        "intervalMin": job.interval_min,
        "calendar": {
            "kind": calendar_kind(job),
            "minuteOfDay": job.cal_time_min,
            "weekday": job.cal_weekdays,
            "monthday": job.cal_monthday,
        },
        "rtDebounceSecs": job.rt_debounce_secs,
        "includeHidden": job.include_hidden,
        "ignore": job.ignore,
        "enabled": job.enabled,
        "runBefore": job.run_before,
        "runAfter": job.run_after,
        "lastRun": job.last_run,
        "activeFromMin": job.active_from_min,
        "activeToMin": job.active_to_min,
        "catchUp": job.catch_up,
        "moveFiles": job.move_files,
        "maxDelete": job.max_delete,
        "maxDeletePct": job.max_delete_pct,
        "useRecycleBin": job.use_recycle_bin,
        "lastResult": result.map(result_json),
        "schedule": schedule_text(job),
        "runningTask": running_task,
    })
}

fn result_json(result: &JobResult) -> Value {
    json!({
        "timeMs": result.when.saturating_mul(1000),
        "aToB": result.a_to_b,
        "bToA": result.b_to_a,
        "deleted": result.deleted,
        "conflicts": result.conflicts,
        "errors": result.errors,
        "note": result.note,
    })
}

fn calendar_kind(job: &SyncJob) -> &'static str {
    if job.cal_monthday != 0 {
        CALENDAR_MONTHLY
    } else if job.cal_weekdays != 0 {
        CALENDAR_WEEKLY
    } else {
        CALENDAR_DAILY
    }
}

/// The desktop job list's trigger description (menus_sync_jobs).
pub(super) fn schedule_text(job: &SyncJob) -> String {
    match job.trigger {
        Trigger::Manual => "manuell".to_string(),
        Trigger::Interval => format!("alle {} min", job.interval_min),
        Trigger::Calendar => {
            let time = min_to_hm(job.cal_time_min);
            if job.cal_monthday != 0 {
                format!("monatl. am {}. um {time}", job.cal_monthday)
            } else if job.cal_weekdays == 0 {
                format!("täglich {time}")
            } else {
                const DAYS: [&str; 7] = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"];
                let days: Vec<&str> = (0..7)
                    .filter(|day| (job.cal_weekdays >> day) & 1 == 1)
                    .map(|day| DAYS[day])
                    .collect();
                format!("{} {time}", days.join(","))
            }
        }
        Trigger::RealTime => format!("Echtzeit (+{}s)", job.rt_debounce_secs),
        Trigger::OnStartup => "beim Start".to_string(),
        Trigger::OnConnect => {
            if job.connect_match.is_empty() {
                "bei USB/Gerät".to_string()
            } else {
                format!("bei Gerät „{}“", job.connect_match)
            }
        }
    }
}

/// Lays the draft over `editor`. Malformed values are collected per field;
/// numbers may arrive as JSON numbers or as half-typed text.
pub(super) fn apply_draft(editor: &mut JobEditor, draft: &Map<String, Value>) -> FieldErrors {
    let mut errors = FieldErrors::new();
    let text = |key: &str, slot: &mut String| {
        if let Some(value) = draft.get(key).and_then(Value::as_str) {
            *slot = value.to_string();
        }
    };
    text("name", &mut editor.name);
    text("source", &mut editor.source);
    text("target", &mut editor.target);
    text("runBefore", &mut editor.run_before);
    text("runAfter", &mut editor.run_after);

    let flag = |key: &str, slot: &mut bool| {
        if let Some(value) = draft.get(key).and_then(Value::as_bool) {
            *slot = value;
        }
    };
    flag("includeHidden", &mut editor.include_hidden);
    flag("enabled", &mut editor.enabled);
    flag("catchUp", &mut editor.catch_up);
    flag("moveFiles", &mut editor.move_files);
    flag("useRecycleBin", &mut editor.use_recycle_bin);

    let number = |key: &str, slot: &mut String| match draft.get(key) {
        Some(Value::Number(value)) => *slot = value.to_string(),
        Some(Value::String(value)) => *slot = value.clone(),
        _ => {}
    };
    number("retainDays", &mut editor.retain_days);
    number("intervalMin", &mut editor.interval_min);
    number("rtDebounceSecs", &mut editor.rt_debounce);
    number("maxDelete", &mut editor.max_delete);
    number("maxDeletePct", &mut editor.max_delete_pct);

    enum_field(
        draft,
        "direction",
        &mut errors,
        &mut editor.direction,
        Direction::parse,
    );
    enum_field(
        draft,
        "conflict",
        &mut errors,
        &mut editor.conflict,
        ConflictMode::parse,
    );
    enum_field(
        draft,
        "deletePolicy",
        &mut errors,
        &mut editor.delete_policy,
        DeletePolicy::parse,
    );
    enum_field(
        draft,
        "compare",
        &mut errors,
        &mut editor.compare,
        CompareMode::parse,
    );
    enum_field(
        draft,
        "versioning",
        &mut errors,
        &mut editor.versioning_scheme,
        VersioningScheme::parse,
    );
    enum_field(
        draft,
        "trigger",
        &mut errors,
        &mut editor.trigger,
        Trigger::parse,
    );

    if let Some(ignore) = draft.get("ignore").and_then(Value::as_array) {
        let patterns: Vec<&str> = ignore.iter().filter_map(Value::as_str).collect();
        editor.ignore = patterns.join("\n");
    }
    minute_field(
        draft,
        "activeFromMin",
        &mut errors,
        &mut editor.active_from,
        "Beginn der aktiven Zeit ist keine gültige Uhrzeit.",
    );
    minute_field(
        draft,
        "activeToMin",
        &mut errors,
        &mut editor.active_to,
        "Ende der aktiven Zeit ist keine gültige Uhrzeit.",
    );
    if let Some(calendar) = draft.get("calendar").and_then(Value::as_object) {
        apply_calendar(editor, calendar, &mut errors);
    }
    errors
}

fn enum_field<T>(
    draft: &Map<String, Value>,
    key: &str,
    errors: &mut FieldErrors,
    slot: &mut T,
    parse: fn(&str) -> Option<T>,
) {
    let Some(value) = draft.get(key).and_then(Value::as_str) else {
        return;
    };
    match parse(value) {
        Some(parsed) => *slot = parsed,
        None => {
            errors.insert(key.to_string(), format!("Unbekannter Wert „{value}“."));
        }
    }
}

fn minute_field(
    draft: &Map<String, Value>,
    key: &str,
    errors: &mut FieldErrors,
    slot: &mut String,
    message: &str,
) {
    let Some(value) = draft.get(key) else {
        return;
    };
    match value
        .as_i64()
        .filter(|minute| (0..24 * 60).contains(minute))
    {
        Some(minute) => *slot = min_to_hm(minute as i32),
        None if value.is_null() => {}
        None => {
            errors.insert(key.to_string(), message.to_string());
        }
    }
}

/// `calendar.weekday` is the desktop bitmask (bit0 = Mo … bit6 = So).
fn apply_calendar(editor: &mut JobEditor, calendar: &Map<String, Value>, errors: &mut FieldErrors) {
    let mut error = |message: &str| {
        errors.insert("calendar".to_string(), message.to_string());
    };
    if let Some(minute) = calendar.get("minuteOfDay") {
        match minute
            .as_i64()
            .filter(|minute| (0..24 * 60).contains(minute))
        {
            Some(minute) => editor.cal_time = min_to_hm(minute as i32),
            None => return error("Uhrzeit muss als HH:MM angegeben werden."),
        }
    }
    let weekday = calendar.get("weekday").and_then(Value::as_u64);
    let monthday = calendar.get("monthday").and_then(Value::as_u64);
    match calendar.get("kind").and_then(Value::as_str) {
        Some(CALENDAR_DAILY) => {
            editor.cal_weekdays = 0;
            editor.cal_monthday = "0".into();
        }
        Some(CALENDAR_WEEKLY) => match weekday.filter(|mask| (1..=0x7f).contains(mask)) {
            Some(mask) => {
                editor.cal_weekdays = mask as u8;
                editor.cal_monthday = "0".into();
            }
            None => error("Bitte mindestens einen Wochentag wählen."),
        },
        Some(CALENDAR_MONTHLY) => match monthday.filter(|day| (1..=31).contains(day)) {
            Some(day) => editor.cal_monthday = day.to_string(),
            None => error("Tag im Monat muss zwischen 1 und 31 liegen."),
        },
        Some(other) => error(&format!("Unbekannter Zeitplan „{other}“.")),
        None => {}
    }
}

/// Assigns the editor's single German error to the field it concerns.
pub(super) fn field_for_message(message: &str) -> &'static str {
    // Order matters: the active-time messages also contain "Uhrzeit".
    const RULES: [(&str, &str); 23] = [
        ("Quelle und Ziel", "target"),
        ("Ignoriermuster", "ignore"),
        ("Spiegel-Löschungen", "deletePolicy"),
        ("Verschieben ist nur", "moveFiles"),
        ("Aufbewahrung", "retainDays"),
        ("Intervall", "intervalMin"),
        ("Beginn der aktiven Zeit", "activeFromMin"),
        ("Ende der aktiven Zeit", "activeToMin"),
        ("Uhrzeit", "calendar"),
        ("Tag im Monat", "calendar"),
        ("Verzögerung", "rtDebounceSecs"),
        ("Lösch-Schutz in Prozent", "maxDeletePct"),
        ("prozentuale Lösch-Schutz", "maxDeletePct"),
        ("Lösch-Schutz", "maxDelete"),
        ("Zeit-Toleranz", "job"),
        ("Versionen behalten", "job"),
        ("Mindestgröße", "job"),
        ("Maximalgröße", "job"),
        ("Mindestalter", "job"),
        ("Maximales Alter", "job"),
        ("Bandbreite", "job"),
        ("Wiederholung", "job"),
        ("Parallele Übertragungen", "job"),
    ];
    RULES
        .iter()
        .find(|(needle, _)| message.contains(needle))
        .map(|(_, field)| *field)
        .unwrap_or("job")
}
