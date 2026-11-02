//! Presentation acknowledgements originate from Android's actual Bitmap/Surface draw.
use crate::mobile::ApiError;
use crate::screen::core::wire_control::ViewerFrame;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;

pub(super) fn confirm(args: &Value) -> Result<Value, ApiError> {
    let session = super::session_arg(args)?;
    // A non-null offer alone never authorizes a new wire message.
    if !session.presented_supported.load(Ordering::SeqCst) {
        return Ok(json!({}));
    }
    let id = |name: &str| {
        args.get(name)
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .ok_or_else(|| super::invalid(format!("„{name}“ fehlt oder ist keine Bildkennung.")))
    };
    let gop_id = id("gopId")?;
    let frame_id = id("frameId")?;
    if session.latest_gop.load(Ordering::SeqCst) != gop_id {
        return Ok(json!({}));
    }
    session.send(&ViewerFrame::Presented { gop_id, frame_id })?;
    Ok(json!({}))
}
