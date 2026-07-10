use super::types::{SyncJob, Trigger};
use crate::bisync::{CompareMode, ConflictMode, DeletePolicy, Direction, VersioningScheme};
use std::str::FromStr;

/// Strip characters that would break the one-value-per-line format.
pub(super) fn san(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

/// Serialize one job as a `key=value` block (the body of its `.conf` file).
pub(super) fn serialize_kv(j: &SyncJob) -> String {
    let mut s = String::new();
    s.push_str("# Smart Explorer sync job\n");
    s.push_str(&format!("id={}\n", san(&j.id)));
    s.push_str(&format!("name={}\n", san(&j.name)));
    s.push_str(&format!("source={}\n", san(&j.source)));
    s.push_str(&format!("target={}\n", san(&j.target)));
    s.push_str(&format!("direction={}\n", j.direction.as_str()));
    s.push_str(&format!("conflict={}\n", j.conflict.as_str()));
    s.push_str(&format!("retain_days={}\n", j.retain_days));
    s.push_str(&format!("interval_min={}\n", j.interval_min));
    push_bool(&mut s, "include_hidden", j.include_hidden);
    for pat in &j.ignore {
        let pattern = san(pat);
        if !pattern.trim().is_empty() {
            s.push_str(&format!("ignore={pattern}\n"));
        }
    }
    s.push_str(&format!("last_run={}\n", j.last_run));
    push_bool(&mut s, "enabled", j.enabled);
    s.push_str(&format!("trigger={}\n", j.trigger.as_str()));
    s.push_str(&format!("cal_time_min={}\n", j.cal_time_min));
    s.push_str(&format!("cal_weekdays={}\n", j.cal_weekdays));
    s.push_str(&format!("cal_monthday={}\n", j.cal_monthday));
    s.push_str(&format!("rt_debounce_secs={}\n", j.rt_debounce_secs));
    s.push_str(&format!("connect_match={}\n", san(&j.connect_match)));
    s.push_str(&format!("active_from_min={}\n", j.active_from_min));
    s.push_str(&format!("active_to_min={}\n", j.active_to_min));
    push_bool(&mut s, "catch_up", j.catch_up);
    s.push_str(&format!("delete_policy={}\n", j.delete_policy.as_str()));
    push_bool(&mut s, "move_files", j.move_files);
    s.push_str(&format!("compare={}\n", j.compare.as_str()));
    s.push_str(&format!("modify_window_sec={}\n", j.modify_window_sec));
    s.push_str(&format!(
        "versioning_scheme={}\n",
        j.versioning_scheme.as_str()
    ));
    s.push_str(&format!("retain_count={}\n", j.retain_count));
    push_bool(&mut s, "use_recycle_bin", j.use_recycle_bin);
    s.push_str(&format!("max_delete={}\n", j.max_delete));
    s.push_str(&format!("max_delete_pct={}\n", j.max_delete_pct));
    s.push_str(&format!("filter_min_size_kb={}\n", j.filter_min_size_kb));
    s.push_str(&format!("filter_max_size_kb={}\n", j.filter_max_size_kb));
    s.push_str(&format!("filter_max_age_days={}\n", j.filter_max_age_days));
    s.push_str(&format!("filter_min_age_days={}\n", j.filter_min_age_days));
    s.push_str(&format!("bwlimit_kbps={}\n", j.bwlimit_kbps));
    s.push_str(&format!("max_transfers={}\n", j.max_transfers));
    push_bool(&mut s, "atomic_copy", j.atomic_copy);
    push_bool(&mut s, "verify", j.verify);
    s.push_str(&format!("retries={}\n", j.retries));
    s.push_str(&format!("retry_delay_secs={}\n", j.retry_delay_secs));
    s.push_str(&format!("run_before={}\n", san(&j.run_before)));
    s.push_str(&format!("run_after={}\n", san(&j.run_after)));
    s
}

fn push_bool(output: &mut String, key: &str, value: bool) {
    output.push_str(&format!("{key}={}\n", u8::from(value)));
}

/// Parse one `key=value` block. Unknown and missing keys remain
/// forward-compatible, but a malformed known key invalidates the whole job.
pub(super) fn parse_kv_checked(body: &str) -> Result<SyncJob, String> {
    let mut job = SyncJob::new(String::new(), String::new(), String::new());
    job.id.clear();
    job.ignore.clear();
    let mut saw_any = false;
    let mut saw_trigger = false;
    for (index, raw_line) in body.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .map(|(key, value)| (key.trim(), value.trim()))
            .ok_or_else(|| format!("line {} is not key=value", index + 1))?;
        if key.is_empty() {
            return Err(format!("line {} has an empty key", index + 1));
        }
        saw_any = true;
        match key {
            "id" => job.id = value.to_string(),
            "name" => job.name = value.to_string(),
            "source" => job.source = value.to_string(),
            "target" => job.target = value.to_string(),
            "direction" => job.direction = parse_enum(key, value, Direction::parse)?,
            "conflict" => job.conflict = parse_enum(key, value, ConflictMode::parse)?,
            "retain_days" => job.retain_days = parse_num(key, value)?,
            "interval_min" => job.interval_min = parse_num(key, value)?,
            "include_hidden" => job.include_hidden = parse_bool(key, value)?,
            "ignore" if !value.is_empty() => job.ignore.push(value.to_string()),
            "ignore" => {}
            "last_run" => job.last_run = parse_num(key, value)?,
            "enabled" => job.enabled = parse_bool(key, value)?,
            "trigger" => {
                job.trigger = parse_enum(key, value, Trigger::parse)?;
                saw_trigger = true;
            }
            "cal_time_min" => job.cal_time_min = parse_num(key, value)?,
            "cal_weekdays" => job.cal_weekdays = parse_num(key, value)?,
            "cal_monthday" => job.cal_monthday = parse_num(key, value)?,
            "rt_debounce_secs" => job.rt_debounce_secs = parse_num(key, value)?,
            "connect_match" => job.connect_match = value.to_string(),
            "active_from_min" => job.active_from_min = parse_num(key, value)?,
            "active_to_min" => job.active_to_min = parse_num(key, value)?,
            "catch_up" => job.catch_up = parse_bool(key, value)?,
            "delete_policy" => job.delete_policy = parse_enum(key, value, DeletePolicy::parse)?,
            "move_files" => job.move_files = parse_bool(key, value)?,
            "compare" => job.compare = parse_enum(key, value, CompareMode::parse)?,
            "modify_window_sec" => job.modify_window_sec = parse_num(key, value)?,
            "versioning_scheme" => {
                job.versioning_scheme = parse_enum(key, value, VersioningScheme::parse)?
            }
            "retain_count" => job.retain_count = parse_num(key, value)?,
            "use_recycle_bin" => job.use_recycle_bin = parse_bool(key, value)?,
            "max_delete" => job.max_delete = parse_num(key, value)?,
            "max_delete_pct" => job.max_delete_pct = parse_num(key, value)?,
            "filter_min_size_kb" => job.filter_min_size_kb = parse_num(key, value)?,
            "filter_max_size_kb" => job.filter_max_size_kb = parse_num(key, value)?,
            "filter_max_age_days" => job.filter_max_age_days = parse_num(key, value)?,
            "filter_min_age_days" => job.filter_min_age_days = parse_num(key, value)?,
            "bwlimit_kbps" => job.bwlimit_kbps = parse_num(key, value)?,
            "max_transfers" => job.max_transfers = parse_num(key, value)?,
            "atomic_copy" => job.atomic_copy = parse_bool(key, value)?,
            "verify" => job.verify = parse_bool(key, value)?,
            "retries" => job.retries = parse_num(key, value)?,
            "retry_delay_secs" => job.retry_delay_secs = parse_num(key, value)?,
            "run_before" => job.run_before = value.to_string(),
            "run_after" => job.run_after = value.to_string(),
            _ => {}
        }
    }
    if !saw_any {
        return Err("job file contains no settings".into());
    }
    if !saw_trigger && job.interval_min > 0 {
        job.trigger = Trigger::Interval;
    }
    job.validate()
        .map_err(|error| format!("invalid sync job: {error}"))?;
    Ok(job)
}

#[cfg(test)]
pub(super) fn parse_kv(body: &str) -> Option<SyncJob> {
    parse_kv_checked(body).ok()
}

/// Legacy positional-TSV parser used for the one-time import.
pub(super) fn parse_legacy(line: &str) -> Option<SyncJob> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() < 12 {
        return None;
    }
    let mut job = SyncJob::new(
        fields[1].to_string(),
        fields[2].to_string(),
        fields[3].to_string(),
    );
    job.id = fields[0].to_string();
    job.direction = Direction::parse(fields[4])?;
    job.conflict = ConflictMode::parse(fields[5])?;
    job.retain_days = fields[6].parse().ok()?;
    job.interval_min = fields[7].parse().ok()?;
    job.include_hidden = parse_bool("include_hidden", fields[8]).ok()?;
    job.ignore = if fields[9].is_empty() {
        Vec::new()
    } else {
        fields[9].split('\u{1f}').map(str::to_string).collect()
    };
    job.last_run = fields[10].parse().ok()?;
    job.enabled = parse_bool("enabled", fields[11]).ok()?;
    job.trigger = if job.interval_min > 0 {
        Trigger::Interval
    } else {
        Trigger::Manual
    };
    job.validate().ok()?;
    Some(job)
}

fn parse_num<T>(key: &str, value: &str) -> Result<T, String>
where
    T: FromStr,
{
    value
        .parse()
        .map_err(|_| format!("{key} is not a valid number: {value:?}"))
}

fn parse_bool(key: &str, value: &str) -> Result<bool, String> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(format!("{key} must be 0 or 1, got {value:?}")),
    }
}

fn parse_enum<T>(
    key: &str,
    value: &str,
    parse: impl FnOnce(&str) -> Option<T>,
) -> Result<T, String> {
    parse(value).ok_or_else(|| format!("{key} has an unknown value: {value:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SyncJob {
        let mut job = SyncJob::new("Docs".into(), "C:/a".into(), "D:/b".into());
        job.interval_min = 15;
        job.trigger = Trigger::Interval;
        job.retain_days = 7;
        job.include_hidden = false;
        job.ignore = vec!["**/*.tmp".into(), "node_modules/**".into()];
        job.conflict = ConflictMode::NewerWins;
        job.direction = Direction::AtoB;
        job
    }

    #[test]
    fn kv_roundtrip() {
        let job = sample();
        let back = parse_kv_checked(&serialize_kv(&job)).unwrap();
        assert_eq!(back.id, job.id);
        assert_eq!(back.direction, Direction::AtoB);
        assert_eq!(back.conflict, ConflictMode::NewerWins);
        assert_eq!(back.ignore, job.ignore);
        assert_eq!(back.interval_min, 15);
    }

    #[test]
    fn unknown_and_missing_keys_remain_forward_compatible() {
        let body = "id=abc\nname=X\nsource=s\ntarget=t\nfuture_option=42\n";
        let job = parse_kv_checked(body).unwrap();
        assert_eq!(job.direction, Direction::Both);
        assert_eq!(job.retain_days, 30);
    }

    #[test]
    fn malformed_known_or_safety_values_reject_the_whole_job() {
        let prefix = "id=abc\nname=X\nsource=s\ntarget=t\n";
        for setting in [
            "max_delete=invalid",
            "max_delete_pct=101",
            "delete_policy=invalid",
            "use_recycle_bin=maybe",
            "atomic_copy=yes",
            "cal_time_min=1440",
            "filter_min_size_kb=2\nfilter_max_size_kb=1",
        ] {
            let error = parse_kv_checked(&format!("{prefix}{setting}\n")).unwrap_err();
            assert!(!error.is_empty(), "{setting} must fail closed");
        }
    }

    #[test]
    fn malformed_non_assignment_line_is_not_ignored() {
        let body = "id=abc\nname=X\nsource=s\ntarget=t\nmax_delete 5\n";
        assert!(parse_kv_checked(body).unwrap_err().contains("key=value"));
    }
}
