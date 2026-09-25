//! Draft state and validation for the add/edit sync-setup dialog: every
//! `SyncJob` field as an editable value (numbers as text, so a half-typed value
//! does not snap back), converted into a validated `SyncJob` with German
//! user-facing errors.
use std::str::FromStr;

/// Draft state for the add/edit sync-setup dialog. Number fields are kept as
/// strings so a half-typed value doesn't snap back.
pub struct JobEditor {
    /// Some(id) when editing an existing job, None for a new one.
    pub id: Option<String>,
    pub name: String,
    pub source: String,
    pub target: String,
    pub direction: crate::bisync::Direction,
    pub conflict: crate::bisync::ConflictMode,
    pub retain_days: String,
    pub interval_min: String,
    pub include_hidden: bool,
    /// One glob per line.
    pub ignore: String,
    pub enabled: bool,
    // ── Group D: scheduling / triggers ───────────────────────────────────────
    pub trigger: crate::syncjobs::Trigger,
    pub cal_time: String,      // "HH:MM"
    pub cal_weekdays: u8,      // bit0=Mon..bit6=Sun, 0 = every day
    pub cal_monthday: String,  // "0" = use weekdays
    pub rt_debounce: String,   // seconds
    pub connect_match: String, // label/serial/letter wildcard
    pub active_from: String,   // "HH:MM"
    pub active_to: String,     // "HH:MM"
    pub catch_up: bool,
    // ── Group B/C: deletion / move / comparison ──────────────────────────────
    pub delete_policy: crate::bisync::DeletePolicy,
    pub move_files: bool,
    pub compare: crate::bisync::CompareMode,
    pub modify_window: String, // seconds
    // ── Group F: versioning & deletion safety ────────────────────────────────
    pub versioning_scheme: crate::bisync::VersioningScheme,
    pub retain_count: String,
    pub use_recycle_bin: bool,
    pub max_delete: String,
    pub max_delete_pct: String,
    // ── Group G: filters ─────────────────────────────────────────────────────
    pub filter_min_size_kb: String,
    pub filter_max_size_kb: String,
    pub filter_max_age_days: String,
    pub filter_min_age_days: String,
    // ── Groups H/I: bandwidth & reliability ──────────────────────────────────
    pub bwlimit_kbps: String,
    pub max_transfers: String,
    pub atomic_copy: bool,
    pub verify: bool,
    pub retries: String,
    pub retry_delay_secs: String,
    pub run_before: String,
    pub run_after: String,
}

/// Minutes-after-midnight → "HH:MM".
pub fn min_to_hm(m: i32) -> String {
    let m = m.rem_euclid(24 * 60);
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// "HH:MM" (or "H", "HHMM") → minutes after midnight; None if unparseable.
pub fn hm_to_min(s: &str) -> Option<i32> {
    let s = s.trim();
    if let Some((h, m)) = s.split_once(':') {
        let h: i32 = h.trim().parse().ok()?;
        let m: i32 = m.trim().parse().ok()?;
        if (0..24).contains(&h) && (0..60).contains(&m) {
            return Some(h * 60 + m);
        }
        return None;
    }
    // bare hour
    let h: i32 = s.parse().ok()?;
    if (0..24).contains(&h) {
        Some(h * 60)
    } else {
        None
    }
}

impl JobEditor {
    pub fn blank(source: String, target: String) -> Self {
        JobEditor {
            id: None,
            name: String::new(),
            source,
            target,
            direction: crate::bisync::Direction::Both,
            conflict: crate::bisync::ConflictMode::FileLevel,
            retain_days: "30".into(),
            interval_min: "0".into(),
            include_hidden: true,
            ignore: String::new(),
            enabled: true,
            trigger: crate::syncjobs::Trigger::Manual,
            cal_time: "09:00".into(),
            cal_weekdays: 0,
            cal_monthday: "0".into(),
            rt_debounce: "10".into(),
            connect_match: String::new(),
            active_from: "00:00".into(),
            active_to: "00:00".into(),
            catch_up: true,
            delete_policy: crate::bisync::DeletePolicy::Propagate,
            move_files: false,
            compare: crate::bisync::CompareMode::MtimeSize,
            modify_window: "0".into(),
            versioning_scheme: crate::bisync::VersioningScheme::Days,
            retain_count: "0".into(),
            use_recycle_bin: false,
            max_delete: "0".into(),
            max_delete_pct: "0".into(),
            filter_min_size_kb: "0".into(),
            filter_max_size_kb: "0".into(),
            filter_max_age_days: "0".into(),
            filter_min_age_days: "0".into(),
            bwlimit_kbps: "0".into(),
            max_transfers: "0".into(),
            atomic_copy: true,
            verify: false,
            retries: "0".into(),
            retry_delay_secs: "2".into(),
            run_before: String::new(),
            run_after: String::new(),
        }
    }

    pub fn from_job(j: &crate::syncjobs::SyncJob) -> Self {
        JobEditor {
            id: Some(j.id.clone()),
            name: j.name.clone(),
            source: j.source.clone(),
            target: j.target.clone(),
            direction: j.direction,
            conflict: j.conflict,
            retain_days: j.retain_days.to_string(),
            interval_min: j.interval_min.to_string(),
            include_hidden: j.include_hidden,
            ignore: j.ignore.join("\n"),
            enabled: j.enabled,
            trigger: j.trigger,
            cal_time: min_to_hm(j.cal_time_min),
            cal_weekdays: j.cal_weekdays,
            cal_monthday: j.cal_monthday.to_string(),
            rt_debounce: j.rt_debounce_secs.to_string(),
            connect_match: j.connect_match.clone(),
            active_from: min_to_hm(j.active_from_min),
            active_to: min_to_hm(j.active_to_min),
            catch_up: j.catch_up,
            delete_policy: j.delete_policy,
            move_files: j.move_files,
            compare: j.compare,
            modify_window: j.modify_window_sec.to_string(),
            versioning_scheme: j.versioning_scheme,
            retain_count: j.retain_count.to_string(),
            use_recycle_bin: j.use_recycle_bin,
            max_delete: j.max_delete.to_string(),
            max_delete_pct: j.max_delete_pct.to_string(),
            filter_min_size_kb: j.filter_min_size_kb.to_string(),
            filter_max_size_kb: j.filter_max_size_kb.to_string(),
            filter_max_age_days: j.filter_max_age_days.to_string(),
            filter_min_age_days: j.filter_min_age_days.to_string(),
            bwlimit_kbps: j.bwlimit_kbps.to_string(),
            max_transfers: j.max_transfers.to_string(),
            atomic_copy: j.atomic_copy,
            verify: j.verify,
            retries: j.retries.to_string(),
            retry_delay_secs: j.retry_delay_secs.to_string(),
            run_before: j.run_before.clone(),
            run_after: j.run_after.clone(),
        }
    }
}

impl JobEditor {
    pub fn build_sync_job(
        &self,
        existing: Option<&crate::syncjobs::SyncJob>,
    ) -> Result<crate::syncjobs::SyncJob, String> {
        let source = self.source.as_str();
        let target = self.target.as_str();
        validate_endpoints(source, target)?;
        validate_ignore_patterns(&self.ignore)?;
        if self.direction == crate::bisync::Direction::Both
            && self.delete_policy == crate::bisync::DeletePolicy::Mirror
        {
            return Err("Spiegel-Löschungen benötigen eine eindeutige Sync-Richtung.".into());
        }
        if self.direction == crate::bisync::Direction::Both && self.move_files {
            return Err("Verschieben ist nur bei einer einseitigen Sync-Richtung möglich.".into());
        }

        let retain_days = parse_number(&self.retain_days, "Aufbewahrung")?;
        let interval_min = parse_number(&self.interval_min, "Intervall")?;
        if self.trigger == crate::syncjobs::Trigger::Interval && interval_min == 0 {
            return Err("Das Intervall muss mindestens eine Minute betragen.".into());
        }
        let cal_time_min = hm_to_min(&self.cal_time)
            .ok_or_else(|| "Uhrzeit muss als HH:MM angegeben werden.".to_string())?;
        let cal_monthday: u8 = parse_number(&self.cal_monthday, "Tag im Monat")?;
        if cal_monthday > 31 {
            return Err("Tag im Monat muss zwischen 0 und 31 liegen.".into());
        }
        let rt_debounce_secs = parse_number(&self.rt_debounce, "Verzögerung")?;
        let active_from_min = hm_to_min(&self.active_from)
            .ok_or_else(|| "Beginn der aktiven Zeit ist keine gültige Uhrzeit.".to_string())?;
        let active_to_min = hm_to_min(&self.active_to)
            .ok_or_else(|| "Ende der aktiven Zeit ist keine gültige Uhrzeit.".to_string())?;
        let modify_window_sec = parse_number(&self.modify_window, "Zeit-Toleranz")?;
        let retain_count = parse_number(&self.retain_count, "Versionen behalten")?;
        let max_delete = parse_number(&self.max_delete, "Lösch-Schutz")?;
        let max_delete_pct: u8 = parse_number(&self.max_delete_pct, "Lösch-Schutz in Prozent")?;
        if max_delete_pct > 100 {
            return Err("Der prozentuale Lösch-Schutz darf höchstens 100 sein.".into());
        }
        let filter_min_size_kb = parse_number(&self.filter_min_size_kb, "Mindestgröße")?;
        let filter_max_size_kb = parse_number(&self.filter_max_size_kb, "Maximalgröße")?;
        if filter_max_size_kb > 0 && filter_min_size_kb > filter_max_size_kb {
            return Err("Die Mindestgröße darf nicht über der Maximalgröße liegen.".into());
        }
        let filter_max_age_days = parse_number(&self.filter_max_age_days, "Maximales Alter")?;
        let filter_min_age_days = parse_number(&self.filter_min_age_days, "Mindestalter")?;
        if filter_max_age_days > 0 && filter_min_age_days > filter_max_age_days {
            return Err("Das Mindestalter darf nicht über dem Maximalalter liegen.".into());
        }
        let bwlimit_kbps = parse_number(&self.bwlimit_kbps, "Bandbreite")?;
        let max_transfers = parse_number(&self.max_transfers, "Parallele Übertragungen")?;
        let retries = parse_number(&self.retries, "Wiederholungen")?;
        if retries > u32::MAX as u64 {
            return Err("Die Zahl der Wiederholungen ist zu groß.".into());
        }
        let retry_delay_secs = parse_number(&self.retry_delay_secs, "Wiederholungspause")?;

        let name = if self.name.trim().is_empty() {
            source
                .trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .filter(|name| !name.is_empty())
                .unwrap_or("Sync")
                .to_string()
        } else {
            self.name.trim().to_string()
        };
        let mut job = existing.cloned().unwrap_or_else(|| {
            crate::syncjobs::SyncJob::new(name.clone(), source.to_string(), target.to_string())
        });
        job.name = name;
        job.source = source.to_string();
        job.target = target.to_string();
        job.direction = self.direction;
        job.conflict = self.conflict;
        job.retain_days = retain_days;
        job.interval_min = interval_min;
        job.include_hidden = self.include_hidden;
        job.ignore = self
            .ignore
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect();
        job.enabled = self.enabled;
        job.trigger = self.trigger;
        job.cal_time_min = cal_time_min;
        job.cal_weekdays = self.cal_weekdays;
        job.cal_monthday = cal_monthday;
        job.rt_debounce_secs = rt_debounce_secs;
        job.connect_match = self.connect_match.trim().to_string();
        job.active_from_min = active_from_min;
        job.active_to_min = active_to_min;
        job.catch_up = self.catch_up;
        job.delete_policy = self.delete_policy;
        job.move_files = self.move_files && self.direction != crate::bisync::Direction::Both;
        job.compare = self.compare;
        job.modify_window_sec = modify_window_sec;
        job.versioning_scheme = self.versioning_scheme;
        job.retain_count = retain_count;
        job.use_recycle_bin = self.use_recycle_bin;
        job.max_delete = max_delete;
        job.max_delete_pct = max_delete_pct;
        job.filter_min_size_kb = filter_min_size_kb;
        job.filter_max_size_kb = filter_max_size_kb;
        job.filter_max_age_days = filter_max_age_days;
        job.filter_min_age_days = filter_min_age_days;
        job.bwlimit_kbps = bwlimit_kbps;
        job.max_transfers = max_transfers;
        job.atomic_copy = self.atomic_copy;
        job.verify = self.verify;
        job.retries = retries;
        job.retry_delay_secs = retry_delay_secs;
        job.run_before = self.run_before.trim().to_string();
        job.run_after = self.run_after.trim().to_string();
        job.validate()
            .map_err(|error| format!("Ungültiges Setup: {error}"))?;
        Ok(job)
    }
}

fn parse_number<T>(raw: &str, label: &str) -> Result<T, String>
where
    T: FromStr,
{
    raw.trim()
        .parse()
        .map_err(|_| format!("{label} enthält keine gültige nichtnegative Zahl."))
}

fn validate_ignore_patterns(raw: &str) -> Result<(), String> {
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in raw.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let glob = globset::Glob::new(pattern)
            .map_err(|error| format!("Ungültiges Ignoriermuster „{pattern}“: {error}"))?;
        builder.add(glob);
    }
    builder
        .build()
        .map(|_| ())
        .map_err(|error| format!("Ignoriermuster konnten nicht kompiliert werden: {error}"))
}

fn validate_endpoints(source: &str, target: &str) -> Result<(), String> {
    crate::connect::validate_sync_endpoints(source, target)
}

#[cfg(test)]
mod tests {
    use super::{validate_endpoints, validate_ignore_patterns, JobEditor};

    #[test]
    fn rejects_invalid_glob_instead_of_silently_skipping_it() {
        let error = validate_ignore_patterns("good/**\n[").unwrap_err();
        assert!(error.contains("["));
    }

    #[test]
    fn rejects_equal_and_nested_endpoints_without_prefix_confusion() {
        assert!(validate_endpoints("/data", "/data").is_err());
        assert!(validate_endpoints("/data", "/data/backup").is_err());
        assert!(validate_endpoints("/data", "/database").is_ok());
        assert!(validate_endpoints("sftp://host/data", "sftp://host/data/sub").is_err());
    }

    #[test]
    fn malformed_delete_guard_and_ambiguous_mirror_are_not_saved() {
        let mut editor = JobEditor::blank("/source".into(), "/target".into());
        editor.max_delete = "not-a-number".into();
        assert!(editor.build_sync_job(None).is_err());

        editor.max_delete = "100".into();
        editor.delete_policy = crate::bisync::DeletePolicy::Mirror;
        assert!(editor.build_sync_job(None).is_err());
    }
}
