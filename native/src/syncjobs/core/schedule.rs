use super::types::{SyncJob, Trigger};

pub(super) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Minutes after local midnight for a unix timestamp.
fn local_min_of_day(now: i64) -> i32 {
    use chrono::{Local, TimeZone, Timelike};
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

impl SyncJob {
    /// Timer-due now? Honours the trigger kind and the active-hours window.
    /// Event triggers (RealTime/OnStartup/OnConnect) are driven by the daemon,
    /// not this timer check, so they return false here.
    pub fn due(&self, now: i64) -> bool {
        if !self.enabled || !self.active_now(now) {
            return false;
        }
        match self.trigger {
            Trigger::Interval => {
                self.interval_min > 0 && (now - self.last_run) >= (self.interval_min as i64 * 60)
            }
            Trigger::Calendar => match self.last_occurrence(now) {
                Some(occ) => {
                    if self.last_run >= occ {
                        false
                    } else if self.catch_up {
                        true
                    } else {
                        // No catch-up: only fire close to the scheduled instant
                        // (within one daemon check window's grace, about 2 min).
                        (now - occ) <= 120
                    }
                }
                None => false,
            },
            _ => false,
        }
    }

    /// Is `now` inside this job's active-hours window? (true when no window set).
    pub fn active_now(&self, now: i64) -> bool {
        within_window(local_min_of_day(now), self.active_from_min, self.active_to_min)
    }

    /// Does `day` (Mon=0..Sun=6 weekday, plus day-of-month) match this calendar?
    fn day_matches(&self, weekday_mon0: u32, day_of_month: u32) -> bool {
        if self.cal_monthday != 0 {
            return day_of_month == self.cal_monthday as u32;
        }
        if self.cal_weekdays == 0 {
            return true;
        }
        (self.cal_weekdays >> weekday_mon0) & 1 == 1
    }

    /// Unix-seconds of the most recent scheduled occurrence at or before `now`
    /// (searching back up to a year), or None if the calendar never matches.
    fn last_occurrence(&self, now: i64) -> Option<i64> {
        use chrono::{Datelike, Duration, Local, TimeZone};
        let now_dt = Local.timestamp_opt(now, 0).single()?;
        let (h, m) = (
            (self.cal_time_min / 60).clamp(0, 23) as u32,
            (self.cal_time_min % 60).clamp(0, 59) as u32,
        );
        for back in 0..366 {
            let d = (now_dt - Duration::days(back)).date_naive();
            if !self.day_matches(d.weekday().num_days_from_monday(), d.day()) {
                continue;
            }
            let naive = d.and_hms_opt(h, m, 0)?;
            if let Some(inst) = Local.from_local_datetime(&naive).single() {
                let ts = inst.timestamp();
                if ts <= now {
                    return Some(ts);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        j.cal_weekdays = 0;
        j.cal_monthday = 0;
        assert!(j.day_matches(0, 15));
        assert!(j.day_matches(6, 1));
        j.cal_weekdays = 0b0001_0001;
        assert!(j.day_matches(0, 10));
        assert!(j.day_matches(4, 10));
        assert!(!j.day_matches(2, 10));
        j.cal_monthday = 15;
        assert!(j.day_matches(2, 15));
        assert!(!j.day_matches(0, 16));
    }
}
