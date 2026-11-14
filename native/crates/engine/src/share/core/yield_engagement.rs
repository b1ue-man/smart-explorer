//! When the yielding controller (`yield_controller.rs`, E2) yields: its
//! connection's purpose yields, the switch allows it, and either an
//! interactive session runs in the process (`interactive_presence.rs`) or a
//! person's metadata operation to the connection's device runs or ended less
//! than the hold time ago (`metadata_presence.rs`).
//!
//! Hold time (decision of 2026-11-14 on E4). Browsing is a sequence of
//! requests; the hold bridges the gap between two of them, so the next one
//! finds the queue already drained instead of draining it again. It is the
//! larger of two times:
//!
//! - the drain time of the controller: from a loaded round trip `R` to the
//!   round trip `B + T` (base delay plus target, queue at the target) the
//!   window must shrink by the factor `R / (B + T)` (the bottleneck rate is
//!   the same, so window and round trip shrink alike). The controller cuts
//!   at most `W / 2` once per round trip (LEDBAT++ §4.2), so it needs
//!   `n = ⌈log2(R / (B + T))⌉` cuts, one per round trip of at most `R`, and
//!   one more round trip before the first cut, because its current delay is a
//!   MIN filter over samples up to one round trip old (RFC 6817 §2.5):
//!   `(n + 1) · R`. Taken while the queue is still full (the latch in
//!   [`PeerHold::refresh`]), a later, already shorter round trip never cuts
//!   the hold short;
//! - [`NAVIGATION_GAP`], the gap between two navigation steps of a person.
//!
//! Example (measurement M3 of run 43633004343): `wan` with `R` ≈ 2.5 s, `B`
//! ≈ 50 ms, `T` = 25 ms gives `n` = 6 and 17.5 s; `lan` with `R` ≈ 170 ms and
//! `B` ≈ 2 ms gives `n` = 3, 0.7 s, so the navigation gap of 3 s holds.
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::interactive_presence::InteractivePresence;
use super::metadata_presence::PeerActivity;
use super::transport_purpose::PurposeSlot;

/// ASSUMPTION (not measured): a person who browses takes at most 3 s from
/// one listing to the next request (reading the listing, choosing the next
/// folder). A longer gap ends the presence; the next request then pays the
/// drain again.
pub(crate) const NAVIGATION_GAP: Duration = Duration::from_secs(3);

/// The hold after the last metadata operation (module comment): the
/// controller's drain time from `round_trip` with `base` delay and `target`,
/// at least [`NAVIGATION_GAP`].
pub(crate) fn hold_time(
    round_trip: Duration,
    base: Option<Duration>,
    target: Duration,
) -> Duration {
    let goal = base
        .unwrap_or_default()
        .saturating_add(target)
        .as_secs_f64();
    let ratio = round_trip.as_secs_f64() / goal.max(f64::MIN_POSITIVE);
    let cuts = if ratio > 1.0 {
        ratio.log2().ceil().min(f64::from(u16::MAX)) as u32
    } else {
        0
    };
    round_trip
        .saturating_mul(cuts.saturating_add(1))
        .max(NAVIGATION_GAP)
}

/// Whether a controller may yield: its connection's purpose yields and the
/// switch allows it.
#[derive(Clone, Debug)]
pub(crate) struct Engagement {
    pub(crate) slot: Arc<PurposeSlot>,
    pub(crate) presence: &'static InteractivePresence,
    /// Off: plain inner controller ("vorher" of measurements M2 and M3).
    pub(crate) enabled: bool,
}

impl Engagement {
    /// Whether the controller yields now; `peer` is the state of its
    /// device's metadata presence ([`PeerHold::engaged`]).
    pub(crate) fn engaged(&self, peer: bool) -> bool {
        self.enabled && self.slot.yields() && (self.presence.active() || peer)
    }

    /// The metadata activity of the connection's device, once bound.
    pub(crate) fn peer_activity(&self) -> Option<&PeerActivity> {
        self.slot.peer_activity()
    }
}

/// The metadata presence of one connection's device as its controller sees
/// it: engaged while an operation runs and until the hold after the last one.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PeerHold {
    /// The latest end of a hold seen so far; never moved earlier.
    until: Option<Instant>,
    engaged: bool,
}

impl PeerHold {
    /// Updates the state at `now` with the `hold` the controller computes
    /// now. The end only moves later, which latches the hold taken while
    /// the queue was full.
    pub(crate) fn refresh(
        &mut self,
        activity: Option<&PeerActivity>,
        now: Instant,
        hold: Duration,
    ) {
        let Some(activity) = activity else {
            self.engaged = false;
            return;
        };
        let reach = if activity.running() {
            Some(now + hold)
        } else {
            activity.last_end().map(|end| end + hold)
        };
        if let Some(reach) = reach {
            self.until = Some(self.until.map_or(reach, |until| until.max(reach)));
        }
        self.engaged = self.until.is_some_and(|until| now < until);
    }

    /// Whether the device's metadata presence holds the controller now.
    pub(crate) fn engaged(&self) -> bool {
        self.engaged
    }
}
