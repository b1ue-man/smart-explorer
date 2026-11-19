//! Where the pictures of a session go in the viewer (user, 2026-11-19: the
//! statistics showed the host's sent pictures per second while 1–10 arrived):
//! pictures received complete by the decode thread, decoded, presented, the
//! ones left out by reason and the viewer's key frame requests, as totals
//! and per second over the last second. Counting only; nothing here decides.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::screen::core::wire_control::KeyframeReason;

/// A read that returns within this found its record waiting already.
pub const QUEUED_WITHIN: Duration = Duration::from_micros(200);

/// The span the rates per second count over.
const RATE_SPAN: Duration = Duration::from_secs(1);

/// Why a picture was left out before it was shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dropped {
    /// Its group was older than the newest one.
    Stale,
    /// Skipped until the next key frame (a broken group, a stream whose
    /// start could not be decoded, a decoder that did not open).
    AwaitingKeyframe,
    /// The decoder failed on it.
    DecodeError,
    /// Its picture stream broke off (`Broken`; the partial frame is lost).
    Broken,
    /// Not shown: the host's visibility floor did not allow it.
    Floor,
    /// Replaced in the slot or held by a newer one before it was shown.
    Overwritten,
    /// Left out by the preview's shortest gap.
    Gap,
    /// Not decoded: the session runs on the tiles, or the family layout
    /// changed under it.
    NotDecoded,
    /// Decoded, but the chosen „Anzeige“ cannot show it (Codex 16).
    Display,
}

impl Dropped {
    const ALL: [Self; 9] = [
        Self::Stale,
        Self::AwaitingKeyframe,
        Self::DecodeError,
        Self::Broken,
        Self::Floor,
        Self::Overwritten,
        Self::Gap,
        Self::NotDecoded,
        Self::Display,
    ];

    fn index(self) -> usize {
        Self::ALL.iter().position(|d| *d == self).unwrap_or(0)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Stale => "veraltet",
            Self::AwaitingKeyframe => "bis Schlüsselbild",
            Self::DecodeError => "Dekodierfehler",
            Self::Broken => "Strom abgebrochen",
            Self::Floor => "Sichtbarkeitsboden",
            Self::Overwritten => "überschrieben",
            Self::Gap => "Vorschau-Abstand",
            Self::NotDecoded => "nicht dekodiert",
            Self::Display => "Anzeige nicht möglich",
        }
    }
}

/// One counter with its events of the last `RATE_SPAN`.
#[derive(Clone, Debug, Default)]
struct Rate {
    total: u64,
    recent: VecDeque<Instant>,
}

impl Rate {
    fn add(&mut self, now: Instant) {
        self.total += 1;
        self.recent.push_back(now);
        self.trim(now);
    }

    fn trim(&mut self, now: Instant) {
        while self
            .recent
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) > RATE_SPAN)
        {
            self.recent.pop_front();
        }
    }

    fn per_second(&self, now: Instant) -> usize {
        self.recent
            .iter()
            .filter(|at| now.saturating_duration_since(**at) <= RATE_SPAN)
            .count()
    }
}

/// The picture flow of one session.
#[derive(Clone, Debug, Default)]
pub struct PictureFlow {
    received: Rate,
    decoded: Rate,
    presented: Rate,
    dropped: [Rate; 9],
    /// Records that were already waiting when the decode thread asked for
    /// the next event (it came back within `QUEUED_WITHIN`): the backlog in
    /// front of the decode thread.
    queued: Rate,
    /// Key frame requests sent: decode error, join, reset.
    keyframes: [u64; 3],
}

impl PictureFlow {
    /// The last record of a picture reached the decode thread.
    pub fn received(&mut self) {
        self.received.add(Instant::now());
    }

    /// The decoder gave a picture.
    pub fn decoded(&mut self) {
        self.decoded.add(Instant::now());
    }

    /// A new picture was presented (its receipt went out).
    pub fn presented(&mut self) {
        self.presented.add(Instant::now());
    }

    /// A record was waiting when the decode thread read it (`QUEUED_WITHIN`).
    pub fn queued(&mut self) {
        self.queued.add(Instant::now());
    }

    pub fn dropped(&mut self, why: Dropped) {
        self.dropped[why.index()].add(Instant::now());
    }

    /// The viewer sent a key frame request.
    pub fn keyframe(&mut self, reason: KeyframeReason) {
        let index = match reason {
            KeyframeReason::DecodeError => 0,
            KeyframeReason::Join => 1,
            KeyframeReason::Reset => 2,
        };
        self.keyframes[index] += 1;
    }

    /// The lines of the statistics field.
    pub fn lines(&self) -> Vec<String> {
        let now = Instant::now();
        let rate = |rate: &Rate| format!("{}/s", rate.per_second(now));
        let mut lines = vec![format!(
            "Bilder empfangen {} · dekodiert {} · angezeigt {}",
            rate(&self.received),
            rate(&self.decoded),
            rate(&self.presented)
        )];
        let dropped: Vec<String> = Dropped::ALL
            .iter()
            .filter(|why| self.dropped[why.index()].total > 0)
            .map(|why| {
                let counter = &self.dropped[why.index()];
                format!(
                    "{} {} ({}/s)",
                    why.label(),
                    counter.total,
                    counter.per_second(now)
                )
            })
            .collect();
        if !dropped.is_empty() {
            lines.push(format!("Verworfen: {}", dropped.join(", ")));
        }
        lines.push(format!(
            "Rückstau vor dem Dekodieren {} Records ({} gesamt)",
            rate(&self.queued),
            self.queued.total
        ));
        lines.push(self.keyframe_line());
        lines
    }

    fn keyframe_line(&self) -> String {
        let [error, join, reset] = self.keyframes;
        format!(
            "Schlüsselbild-Anfragen {} (Dekodierfehler {error}, Beitritt {join}, Rücksetzung {reset})",
            error + join + reset
        )
    }

    /// The sums of a session for the viewer log; `decode` is p50/p95 of
    /// the decode time per picture in ms.
    pub fn summary(&self, decode: Option<(f64, f64)>) -> String {
        let dropped: Vec<String> = Dropped::ALL
            .iter()
            .map(|why| format!("{} {}", why.label(), self.dropped[why.index()].total))
            .collect();
        let decode = decode.map_or_else(
            || "Dekodierdauer –".to_string(),
            |(p50, p95)| format!("Dekodierdauer p50 {p50:.1} ms, p95 {p95:.1} ms"),
        );
        format!(
            "Bildfluss der Sitzung: empfangen {}, dekodiert {}, angezeigt {}; verworfen: {}; \
             Rückstau {} Records; {decode}; {}",
            self.received.total,
            self.decoded.total,
            self.presented.total,
            dropped.join(", "),
            self.queued.total,
            self.keyframe_line()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review1119_task_picture_flow_counts_per_second_and_in_sum() {
        let mut flow = PictureFlow::default();
        for _ in 0..3 {
            flow.received();
        }
        flow.decoded();
        flow.dropped(Dropped::AwaitingKeyframe);
        flow.dropped(Dropped::AwaitingKeyframe);
        flow.keyframe(KeyframeReason::Reset);
        let lines = flow.lines();
        assert_eq!(
            lines[0],
            "Bilder empfangen 3/s · dekodiert 1/s · angezeigt 0/s"
        );
        assert_eq!(lines[1], "Verworfen: bis Schlüsselbild 2 (2/s)");
        assert_eq!(
            lines[2],
            "Rückstau vor dem Dekodieren 0/s Records (0 gesamt)"
        );
        assert!(lines[3].starts_with("Schlüsselbild-Anfragen 1 "));
        let summary = flow.summary(Some((1.0, 2.5)));
        assert!(summary.contains("Dekodierdauer p50 1.0 ms, p95 2.5 ms"));
        assert!(summary.contains("empfangen 3, dekodiert 1, angezeigt 0"));
        assert!(summary.contains("bis Schlüsselbild 2"));
        assert!(summary.contains("Rücksetzung 1"));
        // Events older than the span leave the rate, not the sum.
        let old = Instant::now().checked_sub(RATE_SPAN * 2).unwrap();
        flow.received.recent.iter_mut().for_each(|at| *at = old);
        assert_eq!(flow.received.per_second(Instant::now()), 0);
        assert_eq!(flow.received.total, 3);
    }
}
