//! Timer schedules of sync jobs: interval and calendar occurrences, the
//! active-hours window. Calendars never skip a day (RV1, Y26): a time that
//! does not exist on a daylight-saving day runs at the next valid instant, a
//! time that exists twice runs at the first one, and a month day beyond the
//! month's end runs on its last day.

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone, Timelike};

use super::types::{SyncJob, Trigger};

/// Without a previous evaluation time a calendar job without catch-up still
/// fires this long after its occurrence (one check interval of slack).
const CALENDAR_GRACE_SECS: i64 = 120;
/// Days searched for the previous or next calendar occurrence.
const SEARCH_DAYS: i64 = 400;

pub(super) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Minutes after local midnight for a unix timestamp.
fn local_min_of_day(now: i64) -> i32 {
    match Local.timestamp_opt(now, 0).single() {
        Some(d) => d.hour() as i32 * 60 + d.minute() as i32,
        None => 0,
    }
}

/// Is `cur` (minutes after midnight) within the active window? `from == to`
/// means "always". A window with `from > to` wraps past midnight.
pub fn within_window(cur: i32, from: i32, to: i32) -> bool {
    if from == to {
        return true;
    }
    if from < to {
        cur >= from && cur < to
    } else {
        cur >= from || cur < to
    }
}

fn days_in_month(date: NaiveDate) -> u32 {
    let (year, month) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    NaiveDate::from_ymd_opt(year, month, 1)
        .and_then(|first| first.pred_opt())
        .map_or(31, |last| last.day())
}

impl SyncJob {
    /// Timer-due now, counting from `last_run` (kept for callers without a
    /// job state). The background worker uses `due_at` with the job state's
    /// last success.
    pub fn due(&self, now: i64) -> bool {
        self.due_at(self.last_run, now, None)
    }

    /// Timer-due now when the schedule counts from `anchor` (last success, or
    /// the job's creation when it never succeeded). `since` is the previous
    /// evaluation: a calendar job without catch-up fires for an occurrence
    /// after it (any tick interval works), without one for an occurrence in
    /// the last two minutes. Event triggers are never timer-due.
    pub fn due_at(&self, anchor: i64, now: i64, since: Option<i64>) -> bool {
        if !self.enabled || !self.active_now(now) {
            return false;
        }
        match self.trigger {
            Trigger::Interval => i64::try_from(self.interval_min)
                .ok()
                .and_then(|minutes| minutes.checked_mul(60))
                .is_some_and(|interval| interval > 0 && now.saturating_sub(anchor) >= interval),
            Trigger::Calendar => match self.last_occurrence(now) {
                Some(occurrence) if occurrence > anchor => {
                    self.catch_up
                        || occurrence
                            > since.unwrap_or_else(|| now.saturating_sub(CALENDAR_GRACE_SECS))
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// When the timer schedule is next due (at the earliest `now`), counting
    /// from `anchor`; `None` for event triggers and calendars that never match.
    pub fn next_due_at(&self, anchor: i64, now: i64) -> Option<i64> {
        if !self.enabled {
            return None;
        }
        match self.trigger {
            Trigger::Interval => {
                let interval = i64::try_from(self.interval_min).ok()?.checked_mul(60)?;
                (interval > 0).then(|| anchor.saturating_add(interval).max(now))
            }
            Trigger::Calendar => match self.last_occurrence(now) {
                Some(occurrence) if occurrence > anchor && self.catch_up => Some(now),
                _ => self.next_occurrence(now),
            },
            _ => None,
        }
    }

    /// Is `now` inside this job's active-hours window? (true when no window set).
    pub fn active_now(&self, now: i64) -> bool {
        within_window(
            local_min_of_day(now),
            self.active_from_min,
            self.active_to_min,
        )
    }

    /// Does `date` match this calendar? A month day beyond the month's end
    /// matches the month's last day.
    fn day_matches(&self, date: NaiveDate) -> bool {
        if self.cal_monthday != 0 {
            let wanted = u32::from(self.cal_monthday).min(days_in_month(date));
            return date.day() == wanted;
        }
        if self.cal_weekdays == 0 {
            return true;
        }
        (self.cal_weekdays >> date.weekday().num_days_from_monday()) & 1 == 1
    }

    /// The instant of this calendar's time on `date`: a missing local time
    /// (spring forward) moves to the next valid minute, a repeated one (fall
    /// back) takes the first.
    fn occurrence_on(&self, date: NaiveDate) -> Option<i64> {
        if !(0..24 * 60).contains(&self.cal_time_min) {
            return None;
        }
        let hour = u32::try_from(self.cal_time_min / 60).ok()?;
        let minute = u32::try_from(self.cal_time_min % 60).ok()?;
        let mut naive = date.and_hms_opt(hour, minute, 0)?;
        // Gaps are at most a few hours wide; probe minute by minute past one.
        for _ in 0..=(4 * 60) {
            if let Some(instant) = Local.from_local_datetime(&naive).earliest() {
                return Some(instant.timestamp());
            }
            naive = naive.checked_add_signed(Duration::minutes(1))?;
        }
        None
    }

    /// Unix-seconds of the most recent scheduled occurrence at or before `now`
    /// (searching back about a year), or None if the calendar never matches.
    fn last_occurrence(&self, now: i64) -> Option<i64> {
        let today = Local.timestamp_opt(now, 0).single()?.date_naive();
        (0..SEARCH_DAYS)
            .filter_map(|back| today.checked_sub_signed(Duration::days(back)))
            .filter(|date| self.day_matches(*date))
            .filter_map(|date| self.occurrence_on(date))
            .find(|occurrence| *occurrence <= now)
    }

    /// The first scheduled occurrence after `now`.
    fn next_occurrence(&self, now: i64) -> Option<i64> {
        let today = Local.timestamp_opt(now, 0).single()?.date_naive();
        (0..SEARCH_DAYS)
            .filter_map(|ahead| today.checked_add_signed(Duration::days(ahead)))
            .filter(|date| self.day_matches(*date))
            .filter_map(|date| self.occurrence_on(date))
            .find(|occurrence| *occurrence > now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
        Local
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(year, month, day)
                    .unwrap()
                    .and_hms_opt(hour, minute, 0)
                    .unwrap(),
            )
            .earliest()
            .unwrap()
            .timestamp()
    }

    #[test]
    fn due_logic_interval() {
        let mut j = SyncJob::new("x".into(), "a".into(), "b".into());
        assert!(!j.due(1000), "manual trigger is never timer-due");
        j.trigger = Trigger::Interval;
        assert!(!j.due(1000), "interval 0 = never due");
        j.interval_min = 10;
        j.last_run = 0;
        assert!(j.due(700), "10 min elapsed since epoch");
        j.last_run = 700;
        assert!(!j.due(900), "only 200s since last run");
        assert!(j.due(700 + 600));
        j.enabled = false;
        assert!(!j.due(99999), "disabled never due");

        j.enabled = true;
        j.interval_min = u64::MAX;
        j.last_run = i64::MIN;
        assert!(!j.due(i64::MAX), "overflowing intervals fail closed");
    }

    #[test]
    fn within_window_logic() {
        assert!(within_window(0, 0, 0));
        assert!(within_window(720, 480, 480));
        assert!(within_window(600, 540, 1020));
        assert!(!within_window(1100, 540, 1020));
        assert!(!within_window(300, 540, 1020));
        assert!(within_window(1380, 1320, 360));
        assert!(within_window(120, 1320, 360));
        assert!(!within_window(720, 1320, 360));
    }

    #[test]
    fn calendar_day_matches() {
        let mut j = SyncJob::new("x".into(), "a".into(), "b".into());
        let monday = NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();
        let friday = NaiveDate::from_ymd_opt(2026, 3, 6).unwrap();
        let wednesday = NaiveDate::from_ymd_opt(2026, 3, 4).unwrap();
        j.cal_weekdays = 0;
        j.cal_monthday = 0;
        assert!(j.day_matches(monday));
        j.cal_weekdays = 0b0001_0001;
        assert!(j.day_matches(monday));
        assert!(j.day_matches(friday));
        assert!(!j.day_matches(wednesday));
        j.cal_monthday = 4;
        assert!(j.day_matches(wednesday));
        assert!(!j.day_matches(friday));
    }

    #[test]
    fn review_task_calendar_month_end_runs_on_the_last_day() {
        let mut j = SyncJob::new("x".into(), "a".into(), "b".into());
        j.cal_monthday = 31;
        assert!(j.day_matches(NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()));
        assert!(j.day_matches(NaiveDate::from_ymd_opt(2026, 4, 30).unwrap()));
        assert!(!j.day_matches(NaiveDate::from_ymd_opt(2026, 4, 29).unwrap()));
        assert!(j.day_matches(NaiveDate::from_ymd_opt(2026, 5, 31).unwrap()));
    }

    #[test]
    fn review_task_calendar_never_skips_a_day() {
        let mut j = SyncJob::new("x".into(), "a".into(), "b".into());
        j.trigger = Trigger::Calendar;
        j.cal_time_min = 2 * 60 + 30;
        // Every day of a year has exactly one occurrence, whatever the
        // local daylight-saving rules are.
        let mut day = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        for _ in 0..365 {
            assert!(j.occurrence_on(day).is_some(), "occurrence on {day}");
            day = day.succ_opt().unwrap();
        }
    }

    #[test]
    fn review_task_calendar_without_catch_up_uses_the_previous_check() {
        let mut j = SyncJob::new("x".into(), "a".into(), "b".into());
        j.trigger = Trigger::Calendar;
        j.cal_time_min = 9 * 60;
        j.catch_up = false;
        let occurrence = local(2026, 6, 10, 9, 0);
        let anchor = occurrence - 86_400;
        // A long check interval: the previous check was before 09:00.
        let late = occurrence + 3_000;
        assert!(j.due_at(anchor, late, Some(occurrence - 300)));
        assert!(!j.due_at(anchor, late, Some(occurrence + 60)));
        // Without a previous check only the short grace applies.
        assert!(!j.due_at(anchor, late, None));
        assert!(j.due_at(anchor, occurrence + 60, None));
        // Done since the occurrence.
        assert!(!j.due_at(occurrence + 10, late, Some(occurrence - 300)));
    }

    #[test]
    fn review_task_next_due_times() {
        let mut j = SyncJob::new("x".into(), "a".into(), "b".into());
        j.trigger = Trigger::Interval;
        j.interval_min = 30;
        assert_eq!(j.next_due_at(1_000, 1_100), Some(1_000 + 1_800));
        assert_eq!(j.next_due_at(1_000, 9_000), Some(9_000));
        j.trigger = Trigger::Calendar;
        j.cal_time_min = 9 * 60;
        let now = local(2026, 6, 10, 10, 0);
        assert_eq!(j.next_due_at(now, now), Some(local(2026, 6, 11, 9, 0)));
        j.trigger = Trigger::RealTime;
        assert_eq!(j.next_due_at(0, now), None);
    }
}
