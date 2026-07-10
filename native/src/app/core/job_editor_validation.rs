use super::*;
use std::str::FromStr;

impl JobEditor {
    pub(in crate::app) fn build_sync_job(
        &self,
        existing: Option<&crate::syncjobs::SyncJob>,
    ) -> Result<crate::syncjobs::SyncJob, String> {
        let source = self.source.trim();
        let target = self.target.trim();
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
    if source.is_empty() || target.is_empty() {
        return Err("Quelle und Ziel dürfen nicht leer sein.".into());
    }
    let source_key = endpoint_key(source);
    let target_key = endpoint_key(target);
    if source_key == target_key
        || is_path_prefix(&source_key, &target_key)
        || is_path_prefix(&target_key, &source_key)
    {
        return Err(
            "Quelle und Ziel dürfen weder gleich noch ineinander verschachtelt sein.".into(),
        );
    }

    if !source.contains("://") && !target.contains("://") {
        if let (Ok(source), Ok(target)) =
            (std::fs::canonicalize(source), std::fs::canonicalize(target))
        {
            if source == target || source.starts_with(&target) || target.starts_with(&source) {
                return Err(
                    "Quelle und Ziel verweisen auf denselben oder verschachtelte Ordner.".into(),
                );
            }
        }
    }
    Ok(())
}

fn endpoint_key(endpoint: &str) -> String {
    endpoint
        .trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}

fn is_path_prefix(parent: &str, child: &str) -> bool {
    child
        .strip_prefix(parent)
        .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::{is_path_prefix, validate_endpoints, validate_ignore_patterns, JobEditor};

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
        assert!(is_path_prefix("sftp://host/data", "sftp://host/data/sub"));
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
