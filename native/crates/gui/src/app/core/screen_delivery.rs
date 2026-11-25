//! „Übertragung“ and the measured „Datenrate“ of the desktop viewer (user
//! 2026-11-25: „ich WILL NICHT DASS DU DROSSELST … shadow nutzt tcp oder
//! udt, einstellbar“; Shadow offers „Prefer speed (UDP)“ / „Prefer
//! reliability (TCP)“ and a bit rate „Auto-detect“ with „Restart
//! Auto-Detection“).
//!
//! „Übertragung“ (`Delivery`: „UDP“, the default, or
//! „TCP“) is stored with the viewer's settings
//! (`viewer.json` key `delivery`, absent is „Echtzeit“) and is the same for
//! the toolbar, `se screen view --transport realtime|reliable` and the MCP
//! tool `screen` (action `transport`). Every session opens with it
//! (`StreamRequest::delivery`: the full view and the group view's
//! previews, `screen_pool.rs`); a change in the toolbar goes out at once as
//! `SET_DELIVERY` (`switch`), „Neu messen“ as `REMEASURE` (`remeasure`).
//!
//! Both frames leave only for a host that runs „Übertragung“: its
//! statistics name the mode it runs (`ChainReport::delivery`), which a host
//! without `delivery_v1` never does (`negotiated`). „Datenrate:
//! Automatisch“ is the rate the host measured on the line
//! (`ChainReport::measured_kbps`, its age `measured_age_ms`); with a value
//! of its own the host sends exactly that value (`screen_toolbar_rate.rs`).
use std::io;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

pub use crate::screen::core::delivery::Delivery;
use crate::screen::core::frame_timing::ChainReport;
use crate::screen::core::wire_control::ViewerFrame;

use super::screen_session::{SessionInfo, ViewerSession};

/// „Übertragung“ of this process: unread (`None`) or the stored value.
static DELIVERY: Mutex<Option<Option<Delivery>>> = Mutex::new(None);

/// The stored „Übertragung“ as the file holds it (`None`: never chosen).
pub fn stored() -> Option<Delivery> {
    let mut known = DELIVERY.lock().unwrap_or_else(PoisonError::into_inner);
    *known.get_or_insert_with(|| super::screen_prefs::current().delivery)
}

/// „Übertragung“ in force; never chosen is „UDP“.
pub fn delivery() -> Delivery {
    stored().unwrap_or_default()
}

/// Stores „Übertragung“ at once, under the settings lock all processes
/// share (`screen_prefs::update_stored`).
pub fn set_delivery(delivery: Delivery) -> io::Result<()> {
    *DELIVERY.lock().unwrap_or_else(PoisonError::into_inner) = Some(Some(delivery));
    super::screen_prefs::update_stored(|prefs| prefs.delivery = Some(delivery))
}

/// The name of a mode in the statistics: „Echtzeit“, „Zuverlässig“.
pub fn short_name(delivery: Delivery) -> &'static str {
    match delivery {
        Delivery::Realtime => "UDP",
        Delivery::Reliable => "TCP",
    }
}

fn chain(info: &SessionInfo) -> Option<&ChainReport> {
    info.stats.as_ref().and_then(|stats| stats.chain.as_ref())
}

/// The mode the host reports it runs; `None` before its statistics and from
/// a host without „Übertragung“.
pub fn running(info: &SessionInfo) -> Option<Delivery> {
    chain(info).and_then(|chain| chain.delivery)
}

/// Whether the host of this session takes `SET_DELIVERY` and `REMEASURE`.
pub fn negotiated(info: &SessionInfo) -> bool {
    running(info).is_some()
}

/// The host's statistics came and name no „Übertragung“: a host before it,
/// which neither takes a change nor measures.
pub fn unknown_to_host(info: &SessionInfo) -> bool {
    info.stats.is_some() && running(info).is_none()
}

/// What the host measured of the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Measured {
    pub kbps: u64,
    /// How long ago, in ms; `None` when the host did not say.
    pub age_ms: Option<u64>,
}

/// The host's last measurement in `chain`.
pub fn measured(chain: &ChainReport) -> Option<Measured> {
    chain.measured_kbps.map(|kbps| Measured {
        kbps,
        age_ms: chain.measured_age_ms,
    })
}

/// The host's last measurement in the session's statistics.
pub fn measured_in(info: &SessionInfo) -> Option<Measured> {
    chain(info).and_then(measured)
}

/// kbit/s as Mbit/s with one decimal and the German comma (`48,2`).
pub fn mbit_text(kbps: u64) -> String {
    format!("{:.1}", kbps as f64 / 1000.0).replace('.', ",")
}

/// „Automatisch“ of „Datenrate“ with what the host measured; `negotiated`:
/// the host measures, but has no result yet.
pub fn automatic_text(measured: Option<Measured>, negotiated: bool) -> String {
    match (measured, negotiated) {
        (Some(measured), _) => format!(
            "Automatisch (gemessen: {} Mbit/s)",
            mbit_text(measured.kbps)
        ),
        (None, true) => "Automatisch (noch nicht gemessen)".into(),
        (None, false) => "Automatisch".into(),
    }
}

/// The statistics lines of `chain`: the mode the host runs and the rate it
/// measured, „nicht gemeldet“ from a host before „Übertragung“.
pub fn stats_lines(chain: &ChainReport) -> Vec<String> {
    let Some(delivery) = chain.delivery else {
        return vec!["Übertragung nicht gemeldet (ältere Version)".into()];
    };
    let rate = match measured(chain) {
        Some(Measured {
            kbps,
            age_ms: Some(age),
        }) => format!(
            "Datenrate gemessen {} Mbit/s (vor {} s)",
            mbit_text(kbps),
            age / 1000
        ),
        Some(Measured { kbps, age_ms: None }) => {
            format!("Datenrate gemessen {} Mbit/s", mbit_text(kbps))
        }
        None => "Datenrate noch nicht gemessen".into(),
    };
    vec![format!("Übertragung {}", short_name(delivery)), rate]
}

/// The closed list's text of „Übertragung“: the choice, and how the host
/// stands with it (the words of `screen_toolbar_state::list_text`).
/// `pending`: chosen in this window and not answered yet.
pub fn list_text(
    chosen: Delivery,
    running: Option<Delivery>,
    pending: bool,
    unknown: bool,
) -> String {
    let chosen_label = chosen.label();
    if unknown {
        return format!("Übertragung: {chosen_label} → Gerät kennt die Wahl nicht");
    }
    match running {
        Some(running) if running != chosen && pending => {
            format!("Übertragung: {chosen_label} (wird angewendet …)")
        }
        Some(running) if running != chosen => format!(
            "Übertragung: {chosen_label} → läuft: {}",
            short_name(running)
        ),
        _ => format!("Übertragung: {chosen_label}"),
    }
}

/// How long „Neu messen“ shows „misst die Leitung …“ at most (assumption,
/// not measured: the host's measurement and its next statistics take less;
/// after it the toolbar shows the rate the statistics name, as before).
pub const REMEASURE_WAIT: Duration = Duration::from_secs(15);

/// Whether a „Neu messen“ pressed `since` ago still waits for its result: no
/// measurement younger than the press is reported and `REMEASURE_WAIT` has
/// not passed.
pub fn remeasure_pending(since: Duration, measured: Option<Measured>) -> bool {
    let newer = measured
        .and_then(|measured| measured.age_ms)
        .is_some_and(|age| Duration::from_millis(age) < since);
    since < REMEASURE_WAIT && !newer
}

/// Why an entry of „Übertragung“ cannot be chosen for this session's host.
pub const UNKNOWN_REASON: &str =
    "das Gerät kennt „Übertragung“ nicht (ältere Version) und überträgt wie bisher";

/// „Übertragung“ chosen in the toolbar: stored, and sent to a host that
/// takes it; every later session opens with it.
pub fn switch(session: Option<&ViewerSession>, delivery: Delivery) {
    super::screen_toolbar_ui::note_save(set_delivery(delivery));
    if let Some(session) = session.filter(|session| negotiated(&session.info())) {
        session.send(ViewerFrame::SetDelivery(delivery));
    }
}

/// „Neu messen“: the host measures the line again and sends with the new
/// rate under „Datenrate: Automatisch“.
pub fn remeasure(session: Option<&ViewerSession>) {
    if let Some(session) = session.filter(|session| negotiated(&session.info())) {
        session.send(ViewerFrame::Remeasure);
    }
}

#[cfg(test)]
#[path = "screen_delivery_tests.rs"]
mod tests;
