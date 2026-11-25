//! „Übertragung“ and „Neu messen“ of the Android viewer (user 2026-11-25,
//! `screen::core::delivery`; the desktop viewer's `screen_delivery.rs`):
//!
//! - `screen.viewOpen {…, delivery?}`: `realtime` („UDP“,
//!   also without the field) or `reliable` („TCP“) as
//!   `StreamRequest::delivery`; anything else is refused.
//! - `screen.viewSetDelivery {session, delivery}` → `{"switched": bool}`:
//!   the core sends `SET_DELIVERY` to a host whose statistics name the mode
//!   it runs (`stats.chain.delivery`, which only a host with `delivery_v1`
//!   reports); `switched: false` for an older host and before the first
//!   statistics, the app keeps the choice for its next session.
//! - `screen.viewRemeasure {session}` → `{"sent": bool}`: the host measures
//!   the line again (`REMEASURE`, same condition); its statistics name the
//!   new rate (`chain.measuredKbps`, `chain.measuredAgeMs`).
use std::sync::atomic::Ordering;

use serde_json::{json, Value};

use super::super::args::{invalid, opt_str};
use super::session_arg;
use crate::mobile::ApiError;
use crate::screen::core::delivery::Delivery;
use crate::screen::core::wire_control::{HostStats, ViewerFrame};

const WHICH: &str = "„delivery“ ist realtime oder reliable.";

/// `delivery` of `screen.viewOpen` and `screen.viewSetDelivery`; `None`
/// without it.
pub(super) fn delivery_arg(args: &Value) -> Result<Option<Delivery>, ApiError> {
    match args.get("delivery") {
        None | Some(Value::Null) => Ok(None),
        Some(_) => opt_str(args, "delivery")
            .and_then(Delivery::parse)
            .map(Some)
            .ok_or_else(|| invalid(WHICH)),
    }
}

/// Whether the host's statistics name the „Übertragung“ it runs: only a
/// host with `delivery_v1` does, and only it takes `SET_DELIVERY` and
/// `REMEASURE` (an older host would end the session on an unknown frame).
pub(super) fn host_runs_delivery(stats: &HostStats) -> bool {
    stats
        .chain
        .as_ref()
        .is_some_and(|chain| chain.delivery.is_some())
}

pub(super) fn set_delivery(args: &Value) -> Result<Value, ApiError> {
    let session = session_arg(args)?;
    let mode = delivery_arg(args)?.ok_or_else(|| invalid(WHICH))?;
    if !session.delivery.load(Ordering::SeqCst) {
        return Ok(json!({ "switched": false }));
    }
    session.send(&ViewerFrame::SetDelivery(mode))?;
    Ok(json!({ "switched": true }))
}

pub(super) fn remeasure(args: &Value) -> Result<Value, ApiError> {
    let session = session_arg(args)?;
    if !session.delivery.load(Ordering::SeqCst) {
        return Ok(json!({ "sent": false }));
    }
    session.send(&ViewerFrame::Remeasure)?;
    Ok(json!({ "sent": true }))
}

#[cfg(test)]
#[path = "screen_view_delivery_tests.rs"]
mod tests;
