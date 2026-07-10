use super::types::{SyncJob, Trigger};
use crate::bisync::{DeletePolicy, Direction};

const MINUTES_PER_DAY: i32 = 24 * 60;
const WEEKDAY_MASK: u8 = 0b0111_1111;
const BYTES_PER_KIB: u64 = 1024;
const MILLIS_PER_SECOND: u64 = 1000;
const MILLIS_PER_DAY: u64 = 86_400 * MILLIS_PER_SECOND;
const MAX_ID_LEN: usize = 128;
const MAX_NAME_LEN: usize = 4096;
const MAX_ENDPOINT_LEN: usize = 65_536;
const MAX_IGNORE_PATTERNS: usize = 4096;
const MAX_PATTERN_LEN: usize = 4096;

impl SyncJob {
    /// Validate every persisted setting before it is allowed to influence a
    /// schedule, endpoint lookup, command, or synchronization run.
    pub fn validate(&self) -> Result<(), String> {
        validate_identity(self)?;
        validate_schedule(self)?;
        validate_modes(self)?;
        validate_filters(self)?;
        validate_conversions(self)?;
        compile_ignore_patterns(&self.ignore).map(|_| ())
    }

    /// Compile all ignore patterns as one checked operation. A bad pattern
    /// invalidates the set; it is never silently omitted.
    pub fn checked_glob_set(&self) -> Result<globset::GlobSet, String> {
        compile_ignore_patterns(&self.ignore)
    }
}

fn validate_identity(job: &SyncJob) -> Result<(), String> {
    if job.id.is_empty() || job.id.len() > MAX_ID_LEN {
        return Err(format!("id must contain 1..={MAX_ID_LEN} safe characters"));
    }
    if !job
        .id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("id contains characters that are unsafe for persistence".into());
    }
    validate_text("name", &job.name, MAX_NAME_LEN, false)?;
    validate_text("source", &job.source, MAX_ENDPOINT_LEN, false)?;
    validate_text("target", &job.target, MAX_ENDPOINT_LEN, false)?;
    validate_text("connect_match", &job.connect_match, MAX_ENDPOINT_LEN, true)?;
    validate_text("run_before", &job.run_before, MAX_ENDPOINT_LEN, true)?;
    validate_text("run_after", &job.run_after, MAX_ENDPOINT_LEN, true)?;

    let source = endpoint_key(&job.source);
    let target = endpoint_key(&job.target);
    if source == target
        || is_endpoint_prefix(&source, &target)
        || is_endpoint_prefix(&target, &source)
    {
        return Err("source and target must be distinct, non-nested endpoints".into());
    }
    Ok(())
}

fn validate_text(field: &str, value: &str, max: usize, allow_empty: bool) -> Result<(), String> {
    if (!allow_empty && value.trim().is_empty()) || value.len() > max {
        let requirement = if allow_empty { "0" } else { "1" };
        return Err(format!("{field} must contain {requirement}..={max} bytes"));
    }
    if value.contains(['\0', '\r', '\n']) {
        return Err(format!("{field} contains a forbidden control character"));
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

fn is_endpoint_prefix(parent: &str, child: &str) -> bool {
    child
        .strip_prefix(parent)
        .is_some_and(|rest| rest.starts_with('/'))
}

fn validate_schedule(job: &SyncJob) -> Result<(), String> {
    if job.last_run < 0 {
        return Err("last_run must be a nonnegative Unix timestamp".into());
    }
    if job.cal_time_min < 0 || job.cal_time_min >= MINUTES_PER_DAY {
        return Err("cal_time_min must be between 0 and 1439".into());
    }
    if job.cal_weekdays & !WEEKDAY_MASK != 0 {
        return Err("cal_weekdays contains bits outside Monday through Sunday".into());
    }
    if job.cal_monthday > 31 {
        return Err("cal_monthday must be between 0 and 31".into());
    }
    if !(0..MINUTES_PER_DAY).contains(&job.active_from_min)
        || !(0..MINUTES_PER_DAY).contains(&job.active_to_min)
    {
        return Err("active-hours minutes must be between 0 and 1439".into());
    }
    if job.trigger == Trigger::Interval && job.interval_min == 0 {
        return Err("interval trigger requires interval_min of at least one".into());
    }
    checked_i64_seconds("interval_min", job.interval_min, 60)?;
    i64::try_from(job.rt_debounce_secs)
        .map_err(|_| "rt_debounce_secs is too large for scheduling".to_string())?;
    Ok(())
}

fn validate_modes(job: &SyncJob) -> Result<(), String> {
    if job.max_delete_pct > 100 {
        return Err("max_delete_pct must be between 0 and 100".into());
    }
    if job.direction == Direction::Both && job.move_files {
        return Err("move_files is valid only for a one-way job".into());
    }
    if job.direction == Direction::Both && job.delete_policy == DeletePolicy::Mirror {
        return Err("mirror deletion is valid only for a one-way job".into());
    }
    Ok(())
}

fn validate_filters(job: &SyncJob) -> Result<(), String> {
    if job.filter_max_size_kb > 0 && job.filter_min_size_kb > job.filter_max_size_kb {
        return Err("filter_min_size_kb must not exceed filter_max_size_kb".into());
    }
    if job.filter_max_age_days > 0 && job.filter_min_age_days > job.filter_max_age_days {
        return Err("filter_min_age_days must not exceed filter_max_age_days".into());
    }
    checked_mul_u64("filter_min_size_kb", job.filter_min_size_kb, BYTES_PER_KIB)?;
    checked_mul_u64("filter_max_size_kb", job.filter_max_size_kb, BYTES_PER_KIB)?;
    checked_age_days("filter_max_age_days", job.filter_max_age_days)?;
    checked_age_days("filter_min_age_days", job.filter_min_age_days)?;
    Ok(())
}

fn validate_conversions(job: &SyncJob) -> Result<(), String> {
    checked_i64_seconds("modify_window_sec", job.modify_window_sec, 1000)?;
    checked_mul_u64("bwlimit_kbps", job.bwlimit_kbps, BYTES_PER_KIB)?;
    usize::try_from(job.max_transfers)
        .map_err(|_| "max_transfers is too large for this platform".to_string())?;
    usize::try_from(job.retain_count)
        .map_err(|_| "retain_count is too large for this platform".to_string())?;
    u32::try_from(job.retries).map_err(|_| "retries must not exceed 4294967295".to_string())?;
    Ok(())
}

fn checked_i64_seconds(field: &str, value: u64, factor: i64) -> Result<i64, String> {
    let value = i64::try_from(value).map_err(|_| format!("{field} is too large"))?;
    value
        .checked_mul(factor)
        .ok_or_else(|| format!("{field} overflows its runtime unit"))
}

fn checked_mul_u64(field: &str, value: u64, factor: u64) -> Result<u64, String> {
    value
        .checked_mul(factor)
        .ok_or_else(|| format!("{field} overflows its runtime unit"))
}

fn checked_age_days(field: &str, days: u64) -> Result<(), String> {
    let millis = days
        .checked_mul(MILLIS_PER_DAY)
        .ok_or_else(|| format!("{field} overflows milliseconds"))?;
    i64::try_from(millis)
        .map(|_| ())
        .map_err(|_| format!("{field} is too large for timestamp arithmetic"))
}

fn compile_ignore_patterns(patterns: &[String]) -> Result<globset::GlobSet, String> {
    if patterns.len() > MAX_IGNORE_PATTERNS {
        return Err(format!(
            "ignore contains more than {MAX_IGNORE_PATTERNS} patterns"
        ));
    }
    let mut builder = globset::GlobSetBuilder::new();
    for (index, raw) in patterns.iter().enumerate() {
        let pattern = raw.trim();
        if pattern.is_empty() {
            return Err(format!("ignore pattern {} is empty", index + 1));
        }
        if pattern.len() > MAX_PATTERN_LEN {
            return Err(format!(
                "ignore pattern {} exceeds {MAX_PATTERN_LEN} bytes",
                index + 1
            ));
        }
        let glob = globset::Glob::new(pattern).map_err(|error| {
            format!(
                "invalid ignore pattern {} ({pattern:?}): {error}",
                index + 1
            )
        })?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|error| format!("ignore patterns could not be compiled: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_job() -> SyncJob {
        SyncJob::new("job".into(), "/source".into(), "/target".into())
    }

    #[test]
    fn rejects_invalid_safety_schedule_and_filter_ranges() {
        let mut job = valid_job();
        job.max_delete_pct = 101;
        assert!(job.validate().unwrap_err().contains("max_delete_pct"));

        let mut job = valid_job();
        job.cal_time_min = 1440;
        assert!(job.validate().unwrap_err().contains("cal_time_min"));

        let mut job = valid_job();
        job.cal_weekdays = 0x80;
        assert!(job.validate().unwrap_err().contains("cal_weekdays"));

        let mut job = valid_job();
        job.cal_monthday = 32;
        assert!(job.validate().unwrap_err().contains("cal_monthday"));

        let mut job = valid_job();
        job.active_from_min = -1;
        assert!(job.validate().unwrap_err().contains("active-hours"));

        let mut job = valid_job();
        job.trigger = Trigger::Interval;
        assert!(job.validate().unwrap_err().contains("interval"));

        let mut job = valid_job();
        job.interval_min = u64::MAX;
        assert!(job.validate().unwrap_err().contains("interval_min"));

        let mut job = valid_job();
        job.rt_debounce_secs = u64::MAX;
        assert!(job.validate().unwrap_err().contains("rt_debounce_secs"));

        let mut job = valid_job();
        job.filter_min_size_kb = 2;
        job.filter_max_size_kb = 1;
        assert!(job.validate().unwrap_err().contains("filter_min_size"));
    }

    #[test]
    fn rejects_overflowing_runtime_conversions() {
        let mut job = valid_job();
        job.modify_window_sec = u64::MAX;
        assert!(job.validate().unwrap_err().contains("modify_window_sec"));

        let mut job = valid_job();
        job.filter_max_age_days = u64::MAX;
        assert!(job.validate().unwrap_err().contains("filter_max_age_days"));

        let mut job = valid_job();
        job.retries = u32::MAX as u64 + 1;
        assert!(job.validate().unwrap_err().contains("retries"));
    }

    #[test]
    fn checked_globs_reject_the_entire_invalid_set() {
        let mut job = valid_job();
        job.ignore = vec!["good/**".into(), "[".into()];
        let error = job.checked_glob_set().unwrap_err();
        assert!(error.contains("pattern 2"));
    }

    #[test]
    fn rejects_equal_or_nested_endpoints() {
        let mut job = valid_job();
        job.target = "/source/backup".into();
        assert!(job.validate().unwrap_err().contains("non-nested"));
    }
}
