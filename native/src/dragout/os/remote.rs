//! Dragging a remote selection out as virtual files (Block F of the
//! transfer-engine plan). Until it lands, the request fails clearly.
use super::DragOutOutcome;
use windows::core::{Error, Result};
use windows::Win32::Foundation::E_NOTIMPL;

pub fn drag_out_remote(_source: crate::transfer::SelectionSource) -> Result<DragOutOutcome> {
    Err(Error::new(
        E_NOTIMPL,
        "Remote-Dateien direkt herausziehen ist noch nicht verfügbar",
    ))
}
