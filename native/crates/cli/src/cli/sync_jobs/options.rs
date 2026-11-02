//! Every field of the desktop sync-setup editor as a `se sync add/edit`
//! option, laid over the shared `JobEditor` draft. Option names are the keys
//! of the saved job file (`sync/jobs/<id>.conf`) and of `se sync show --json`;
//! choice values are the stored values. Numbers and times go into the draft as
//! text, so the editor's own validation and German messages apply unchanged.
use clap::Args;

use crate::bisync::{
    CompareMode, ConflictMode, DeletePolicy, Direction, VersioningScheme, VersionsLocation,
};
use crate::syncjobs::editor::JobEditor;
use crate::syncjobs::Trigger;

pub(super) const DIRECTIONS: [&str; 3] = ["both", "a2b", "b2a"];
pub(super) const CONFLICTS: [&str; 8] = [
    "strict", "newer", "older", "larger", "smaller", "source", "dest", "keepboth",
];
pub(super) const DELETE_POLICIES: [&str; 3] = ["propagate", "mirror", "nodelete"];
pub(super) const COMPARES: [&str; 3] = ["mtimesize", "sizeonly", "checksum"];
pub(super) const VERSIONING: [&str; 4] = ["days", "count", "staggered", "gfs"];
pub(super) const VERSIONS_LOCATIONS: [&str; 2] = ["auto", "appdata"];
pub(super) const TRIGGERS: [&str; 6] = [
    "manual",
    "interval",
    "calendar",
    "realtime",
    "onstartup",
    "onconnect",
];
/// Bit 0 = Monday … bit 6 = Sunday, as `SyncJob::cal_weekdays`.
pub(super) const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

#[derive(Args, Default)]
pub(super) struct JobOptions {
    #[arg(
        long,
        help = "Name shown in the lists (empty: the last folder of side A)"
    )]
    name: Option<String>,
    #[arg(
        long,
        value_name = "LOCATION",
        help = "Side A: absolute local path, sftp:// ftp:// ftps:// webdav:// smb:// endpoint, share:// endpoint, gdrive:/// path, or @label:/path"
    )]
    source: Option<String>,
    #[arg(long, value_name = "LOCATION", help = "Side B, same forms as --source")]
    target: Option<String>,
    #[arg(long, value_parser = DIRECTIONS, help = "both = two-way; a2b/b2a = one-way")]
    direction: Option<String>,
    #[arg(long, value_parser = CONFLICTS, help = "What wins when both sides changed (strict = report a conflict)")]
    conflict: Option<String>,
    #[arg(long, value_parser = DELETE_POLICIES, help = "propagate deletions, mirror (one-way: remove extra files), or never delete")]
    delete_policy: Option<String>,
    #[arg(
        long,
        help = "One-way only: remove the source file after a successful copy (true/false)"
    )]
    move_files: Option<bool>,
    #[arg(long, value_parser = COMPARES, help = "How equal files are recognized")]
    compare: Option<String>,
    #[arg(
        long,
        value_name = "SECONDS",
        help = "Treat time differences up to this many seconds as equal (mtimesize)"
    )]
    modify_window_sec: Option<u64>,
    #[arg(long, value_parser = VERSIONING, help = "How long versions of replaced and deleted files are kept")]
    versioning_scheme: Option<String>,
    #[arg(
        long,
        value_name = "DAYS",
        help = "days: keep versions this many days (0 = forever)"
    )]
    retain_days: Option<u64>,
    #[arg(
        long,
        value_name = "COUNT",
        help = "count: keep this many newest versions (0 = all)"
    )]
    retain_count: Option<u64>,
    #[arg(long, value_parser = VERSIONS_LOCATIONS, help = "auto = .se-versions at the synced folder, else app data; appdata = always app data")]
    versions_location: Option<String>,
    #[arg(
        long,
        help = "Move deleted local files to the recycle bin (true/false)"
    )]
    use_recycle_bin: Option<bool>,
    #[arg(
        long,
        value_name = "FILES",
        help = "Stop a run that would delete more files (0 = no limit)"
    )]
    max_delete: Option<u64>,
    #[arg(
        long,
        value_name = "PERCENT",
        help = "Stop a run that would delete more than this percent of a side (0 = no limit)"
    )]
    max_delete_pct: Option<u8>,
    #[arg(
        long,
        value_name = "FILES",
        help = "The percent stop applies from this many deletions on"
    )]
    max_delete_min: Option<u64>,
    #[arg(long, value_parser = TRIGGERS, help = "What starts the setup automatically")]
    trigger: Option<String>,
    #[arg(long, value_name = "MINUTES", help = "interval: run every N minutes")]
    interval_min: Option<u64>,
    #[arg(long, value_name = "HH:MM", help = "calendar: start time")]
    cal_time: Option<String>,
    #[arg(
        long,
        value_name = "DAYS",
        help = "calendar: comma-separated mon,tue,wed,thu,fri,sat,sun, or daily"
    )]
    cal_weekdays: Option<String>,
    #[arg(
        long,
        value_name = "DAY",
        help = "calendar: day of month 1-31 for a monthly run (0 = use the weekdays)"
    )]
    cal_monthday: Option<u8>,
    #[arg(
        long,
        value_name = "SECONDS",
        help = "realtime: wait after the last change before running"
    )]
    rt_debounce_secs: Option<u64>,
    #[arg(
        long,
        value_name = "SECONDS",
        help = "realtime: longest wait while changes keep coming (0 = automatic)"
    )]
    rt_max_latency_secs: Option<u64>,
    #[arg(
        long,
        value_name = "SECONDS",
        help = "realtime: poll sides without change events this often (0 = never)"
    )]
    rt_poll_secs: Option<u64>,
    #[arg(
        long,
        value_name = "SECONDS",
        help = "Control run that walks the watched side(s) completely (0 = none)"
    )]
    verify_interval_secs: Option<u64>,
    #[arg(
        long,
        value_name = "SECONDS",
        help = "Complete check of a pure target side (0 = none)"
    )]
    verify_target_secs: Option<u64>,
    #[arg(
        long,
        value_name = "PATTERN",
        help = "onconnect: volume label, serial or drive letter, * and ? allowed (empty = any removable drive)"
    )]
    connect_match: Option<String>,
    #[arg(
        long,
        value_name = "HH:MM",
        help = "Run only from this time (equal to --active-to = always)"
    )]
    active_from: Option<String>,
    #[arg(long, value_name = "HH:MM", help = "Run only until this time")]
    active_to: Option<String>,
    #[arg(
        long,
        help = "calendar: run a missed occurrence as soon as possible (true/false)"
    )]
    catch_up: Option<bool>,
    #[arg(long, help = "Include hidden files (true/false)")]
    include_hidden: Option<bool>,
    #[arg(
        long,
        value_name = "GLOB",
        help = "Skip matching relative paths; repeat for several patterns (replaces the list)"
    )]
    ignore: Vec<String>,
    #[arg(long, conflicts_with = "ignore", help = "Remove every ignore pattern")]
    clear_ignore: bool,
    #[arg(
        long,
        value_name = "KB",
        help = "Only files of at least this size (0 = off)"
    )]
    filter_min_size_kb: Option<u64>,
    #[arg(
        long,
        value_name = "KB",
        help = "Only files of at most this size (0 = off)"
    )]
    filter_max_size_kb: Option<u64>,
    #[arg(
        long,
        value_name = "DAYS",
        help = "Only files changed within this many days (0 = off)"
    )]
    filter_max_age_days: Option<u64>,
    #[arg(
        long,
        value_name = "DAYS",
        help = "Only files older than this many days (0 = off)"
    )]
    filter_min_age_days: Option<u64>,
    #[arg(
        long,
        help = "Descend into other file systems mounted inside a side (true/false)"
    )]
    cross_mounts: Option<bool>,
    #[arg(
        long,
        value_name = "KB/S",
        help = "Limit the transfer rate (0 = unlimited)"
    )]
    bwlimit_kbps: Option<u64>,
    #[arg(
        long,
        value_name = "COUNT",
        help = "Parallel transfers (0 = automatic)"
    )]
    max_transfers: Option<u64>,
    #[arg(
        long,
        help = "Copy to a temporary file and rename it when complete (true/false)"
    )]
    atomic_copy: Option<bool>,
    #[arg(long, help = "Check the size after each copy (true/false)")]
    verify: Option<bool>,
    #[arg(
        long,
        value_name = "COUNT",
        help = "Retry a failed transfer this many times"
    )]
    retries: Option<u64>,
    #[arg(long, value_name = "SECONDS", help = "Pause between retries")]
    retry_delay_secs: Option<u64>,
    #[arg(
        long,
        value_name = "COMMAND",
        help = "Command run before a manual or scheduled run; a failure stops the run"
    )]
    run_before: Option<String>,
    #[arg(
        long,
        value_name = "COMMAND",
        help = "Command run after every started run, also after errors"
    )]
    run_after: Option<String>,
    #[arg(
        long,
        value_name = "COMMAND",
        help = "Command run after a canceled or failed run"
    )]
    run_cleanup: Option<String>,
    #[arg(long, help = "Schedule active (true/false)")]
    enabled: Option<bool>,
}

impl JobOptions {
    /// Side A and B as given (before `@label` resolution), for `add`.
    pub(super) fn sides(&self) -> (Option<&str>, Option<&str>) {
        (self.source.as_deref(), self.target.as_deref())
    }

    /// Lays every given option over `editor`; options not given keep the
    /// draft's value (the stored job's, or the defaults of a new one).
    pub(super) fn apply(self, editor: &mut JobEditor) -> Result<(), String> {
        let text = |slot: &mut String, value: Option<String>| {
            if let Some(value) = value {
                *slot = value;
            }
        };
        let number = |slot: &mut String, value: Option<u64>| {
            if let Some(value) = value {
                *slot = value.to_string();
            }
        };
        let flag = |slot: &mut bool, value: Option<bool>| {
            if let Some(value) = value {
                *slot = value;
            }
        };
        text(&mut editor.name, self.name);
        if let Some(source) = self.source {
            editor.source = location(&source)?;
        }
        if let Some(target) = self.target {
            editor.target = location(&target)?;
        }
        choice(&mut editor.direction, self.direction, Direction::parse)?;
        choice(&mut editor.conflict, self.conflict, ConflictMode::parse)?;
        choice(
            &mut editor.delete_policy,
            self.delete_policy,
            DeletePolicy::parse,
        )?;
        flag(&mut editor.move_files, self.move_files);
        choice(&mut editor.compare, self.compare, CompareMode::parse)?;
        number(&mut editor.modify_window, self.modify_window_sec);
        choice(
            &mut editor.versioning_scheme,
            self.versioning_scheme,
            VersioningScheme::parse,
        )?;
        number(&mut editor.retain_days, self.retain_days);
        number(&mut editor.retain_count, self.retain_count);
        choice(
            &mut editor.versions_location,
            self.versions_location,
            VersionsLocation::parse,
        )?;
        flag(&mut editor.use_recycle_bin, self.use_recycle_bin);
        number(&mut editor.max_delete, self.max_delete);
        number(
            &mut editor.max_delete_pct,
            self.max_delete_pct.map(u64::from),
        );
        number(&mut editor.max_delete_min, self.max_delete_min);
        choice(&mut editor.trigger, self.trigger, Trigger::parse)?;
        number(&mut editor.interval_min, self.interval_min);
        text(&mut editor.cal_time, self.cal_time);
        if let Some(days) = self.cal_weekdays {
            editor.cal_weekdays = weekday_mask(&days)?;
        }
        number(&mut editor.cal_monthday, self.cal_monthday.map(u64::from));
        number(&mut editor.rt_debounce, self.rt_debounce_secs);
        number(&mut editor.rt_max_latency, self.rt_max_latency_secs);
        number(&mut editor.rt_poll, self.rt_poll_secs);
        number(&mut editor.verify_interval, self.verify_interval_secs);
        number(&mut editor.verify_target, self.verify_target_secs);
        text(&mut editor.connect_match, self.connect_match);
        text(&mut editor.active_from, self.active_from);
        text(&mut editor.active_to, self.active_to);
        flag(&mut editor.catch_up, self.catch_up);
        flag(&mut editor.include_hidden, self.include_hidden);
        if self.clear_ignore {
            editor.ignore.clear();
        }
        if !self.ignore.is_empty() {
            editor.ignore = self.ignore.join("\n");
        }
        number(&mut editor.filter_min_size_kb, self.filter_min_size_kb);
        number(&mut editor.filter_max_size_kb, self.filter_max_size_kb);
        number(&mut editor.filter_max_age_days, self.filter_max_age_days);
        number(&mut editor.filter_min_age_days, self.filter_min_age_days);
        flag(&mut editor.cross_mounts, self.cross_mounts);
        number(&mut editor.bwlimit_kbps, self.bwlimit_kbps);
        number(&mut editor.max_transfers, self.max_transfers);
        flag(&mut editor.atomic_copy, self.atomic_copy);
        flag(&mut editor.verify, self.verify);
        number(&mut editor.retries, self.retries);
        number(&mut editor.retry_delay_secs, self.retry_delay_secs);
        text(&mut editor.run_before, self.run_before);
        text(&mut editor.run_after, self.run_after);
        text(&mut editor.run_cleanup, self.run_cleanup);
        flag(&mut editor.enabled, self.enabled);
        Ok(())
    }
}

fn choice<T>(
    slot: &mut T,
    value: Option<String>,
    parse: fn(&str) -> Option<T>,
) -> Result<(), String> {
    if let Some(value) = value {
        *slot = parse(&value).ok_or_else(|| format!("unknown value {value:?}"))?;
    }
    Ok(())
}

/// `mon,wed,fri` → the desktop bitmask; `daily` = no weekday chosen (every
/// day, like the desktop editor with no day selected).
pub(super) fn weekday_mask(text: &str) -> Result<u8, String> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("daily") || text.is_empty() {
        return Ok(0);
    }
    let mut mask = 0u8;
    for day in text.split(',').map(str::trim).filter(|day| !day.is_empty()) {
        let index = WEEKDAYS
            .iter()
            .position(|known| known.eq_ignore_ascii_case(day))
            .ok_or_else(|| {
                format!("unknown weekday {day:?}: use mon,tue,wed,thu,fri,sat,sun or daily")
            })?;
        mask |= 1 << index;
    }
    Ok(mask)
}

/// A sync side as the GUI stores it. `@label:/path` of a saved remote becomes
/// its endpoint (the form the picker writes); local paths must be absolute
/// because the background worker has no working directory of the terminal.
pub(super) fn location(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("a sync side must not be empty".into());
    }
    if let Some((connection, path)) = crate::cli::target::saved_shorthand(text)? {
        if !connection.protocol.is_url() {
            return Err(format!(
                "{text}: give the network share path itself (\\\\server\\share\\…)"
            ));
        }
        return Ok(crate::connect::remote_endpoint(&connection, &path));
    }
    let remote = text.contains("://") || text.starts_with("\\\\") || text.starts_with("//");
    if !remote && !std::path::Path::new(text).is_absolute() {
        return Err(format!(
            "{text}: local sync sides need an absolute path (the background worker does not run in this directory)"
        ));
    }
    Ok(text.to_string())
}

#[cfg(test)]
#[path = "options_tests.rs"]
mod tests;
