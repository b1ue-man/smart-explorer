//! The unthrottled rate of a picture stream (user 2026-11-25: „ich WILL
//! NICHT DASS DU DROSSELST … alles drum herum an die verfügbare datenrate
//! anpassen … auch bei änderung“; Shadow's „Auto-detect“: the app measures
//! the connection and sets the bit rate, a new measurement on demand,
//! support.shadow.tech „Configuring Bitrate and Bandwidth in Shadow PC“,
//! read 2026-11-25; Moonlight/Sunshine stream at the rate set, no automatic
//! regulation).
//!
//! The encoder gets the viewer's „Datenrate“ when it set one, else the
//! available rate the last measurement of the line found
//! (`bandwidth_probe`), else, before the first measurement, Moonlight's table
//! of the stream (`model::start_table_bps`); never more than the codec
//! pairing carries. No delay rise, loss, stale stream or refused picture
//! lowers it: the delay controller, the loss target and the queue still run
//! for the statistics, but `current` ignores them. A new measurement (a new
//! route, the viewer's „Neu messen“, the periodic one) replaces the rate,
//! up or down.
use super::model::RateLimit;
use super::RateController;

impl RateController {
    /// From now on no estimate lowers the target.
    pub fn set_unthrottled(&mut self, unthrottled: bool) {
        self.unthrottled = unthrottled;
    }

    pub fn unthrottled(&self) -> bool {
        self.unthrottled
    }

    /// The line's available rate a measurement found, in bit/s.
    pub fn set_measured(&mut self, bps: u64) {
        self.measured_bps = Some(bps.max(super::MIN_BITRATE_BPS));
    }

    pub fn measured_bps(&self) -> Option<u64> {
        self.measured_bps
    }

    /// The unthrottled rate before the ceilings: the viewer's „Datenrate“,
    /// the measured rate, the stream's table, the path's start.
    pub(super) fn unthrottled_bps(&self) -> f64 {
        let bps = self
            .user_ceiling_bps
            .or(self.measured_bps)
            .or(self.table_bps)
            .unwrap_or_else(|| self.path.start_bps());
        bps as f64
    }

    /// What holds the unthrottled rate (`STATS` `chain.rate_limit`); `None`
    /// for the regulated controller.
    pub(super) fn unthrottled_limit(&self) -> Option<RateLimit> {
        if !self.unthrottled {
            return None;
        }
        Some(if self.user_ceiling_bps.is_some() {
            RateLimit::Ceiling
        } else if self.measured_bps.is_some() {
            RateLimit::Measured
        } else {
            RateLimit::Ramp
        })
    }
}

#[cfg(test)]
#[path = "rate_unthrottled_tests.rs"]
mod tests;
