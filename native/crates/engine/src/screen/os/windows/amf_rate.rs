//! The bit rate of a running AMF session (moved out of `amf_encode.rs`,
//! review 2026-11-19 X1/X4, which keeps that file under 500 lines).
//!
//! A lowered rate is set however small the step (C10); a refused lowering
//! fails the picture instead of running above it (the stage opens a fresh
//! encoder at that rate). A refused rate stays the session's reference
//! rate's neighbour: `rate` keeps what the component runs (T1), the refused
//! one is remembered and asked again only for another target (X4, as
//! NVENC's `refused`), and `rate_refused` tells the state of the last rate
//! asked, not every refusal since the start (X1).
use super::*;

impl AmfSession {
    /// Asks the component for `rate` when it differs from the one it runs.
    pub(super) fn retune(&mut self, rate: &RateParams) -> io::Result<()> {
        let names = self.names;
        let lowered = self.rate.lowered_by(rate);
        let changed = self.rate.differs(rate) || lowered;
        if !changed || self.rate_refusal == Some(*rate) {
            return Ok(());
        }
        let previous = self.rate;
        self.rate = *rate;
        let refused = self.apply(rate_settings(names, rate));
        if let Some(ltr) = self.ltr.as_mut() {
            ltr.set_fps(rate.fps);
        }
        // The peak counts in quality VBR (sharp mode) only; CBR may refuse it.
        let rated = [Some(names.target), rate.quality.map(|_| names.peak)];
        let Some(name) = refused.iter().find(|name| rated.contains(&Some(**name))) else {
            self.rate_refusal = None;
            return Ok(());
        };
        // Codex round 5, finding 18: the properties set before the refused
        // one stay on the component; the rate before goes back on all of
        // them, so the component runs what `rate` says it runs.
        self.apply(rate_settings(names, &previous));
        if let Some(ltr) = self.ltr.as_mut() {
            ltr.set_fps(previous.fps);
        }
        self.rate = previous;
        self.rate_refusal = Some(*rate);
        let error = io::Error::other(format!("AMF refused {name}"));
        // Codex round 5, finding 11: a refused lowering within 2 % of the rate
        // it runs stays with it; the stream's byte budget keeps the ceiling.
        rate.refused_lowering(lowered && previous.differs(rate), AMF_NAME, &error)
    }
}
