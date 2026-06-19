/// Draft state for the add/edit sync-setup dialog. Number fields are kept as
/// strings so a half-typed value doesn't snap back.
pub(in crate::app) struct JobEditor {
    /// Some(id) when editing an existing job, None for a new one.
    pub(in crate::app) id: Option<String>,
    pub(in crate::app) name: String,
    pub(in crate::app) source: String,
    pub(in crate::app) target: String,
    pub(in crate::app) direction: crate::bisync::Direction,
    pub(in crate::app) conflict: crate::bisync::ConflictMode,
    pub(in crate::app) retain_days: String,
    pub(in crate::app) interval_min: String,
    pub(in crate::app) include_hidden: bool,
    /// One glob per line.
    pub(in crate::app) ignore: String,
    pub(in crate::app) enabled: bool,
    // ── Group D: scheduling / triggers ───────────────────────────────────────
    pub(in crate::app) trigger: crate::syncjobs::Trigger,
    pub(in crate::app) cal_time: String,      // "HH:MM"
    pub(in crate::app) cal_weekdays: u8,      // bit0=Mon..bit6=Sun, 0 = every day
    pub(in crate::app) cal_monthday: String,  // "0" = use weekdays
    pub(in crate::app) rt_debounce: String,   // seconds
    pub(in crate::app) connect_match: String, // label/serial/letter wildcard
    pub(in crate::app) active_from: String,   // "HH:MM"
    pub(in crate::app) active_to: String,     // "HH:MM"
    pub(in crate::app) catch_up: bool,
    // ── Group B/C: deletion / move / comparison ──────────────────────────────
    pub(in crate::app) delete_policy: crate::bisync::DeletePolicy,
    pub(in crate::app) move_files: bool,
    pub(in crate::app) compare: crate::bisync::CompareMode,
    pub(in crate::app) modify_window: String, // seconds
    // ── Group F: versioning & deletion safety ────────────────────────────────
    pub(in crate::app) versioning_scheme: crate::bisync::VersioningScheme,
    pub(in crate::app) retain_count: String,
    pub(in crate::app) use_recycle_bin: bool,
    pub(in crate::app) max_delete: String,
    pub(in crate::app) max_delete_pct: String,
    // ── Group G: filters ─────────────────────────────────────────────────────
    pub(in crate::app) filter_min_size_kb: String,
    pub(in crate::app) filter_max_size_kb: String,
    pub(in crate::app) filter_max_age_days: String,
    pub(in crate::app) filter_min_age_days: String,
    // ── Groups H/I: bandwidth & reliability ──────────────────────────────────
    pub(in crate::app) bwlimit_kbps: String,
    pub(in crate::app) max_transfers: String,
    pub(in crate::app) atomic_copy: bool,
    pub(in crate::app) verify: bool,
    pub(in crate::app) retries: String,
    pub(in crate::app) retry_delay_secs: String,
    pub(in crate::app) run_before: String,
    pub(in crate::app) run_after: String,
}

/// Minutes-after-midnight → "HH:MM".
pub(in crate::app) fn min_to_hm(m: i32) -> String {
    let m = m.rem_euclid(24 * 60);
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// "HH:MM" (or "H", "HHMM") → minutes after midnight; None if unparseable.
pub(in crate::app) fn hm_to_min(s: &str) -> Option<i32> {
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
    pub(in crate::app) fn blank(source: String, target: String) -> Self {
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

    pub(in crate::app) fn from_job(j: &crate::syncjobs::SyncJob) -> Self {
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
