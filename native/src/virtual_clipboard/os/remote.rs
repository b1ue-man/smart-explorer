//! Remote selections as virtual files on the clipboard (Block F of the
//! transfer-engine plan). Until it lands, the request fails clearly and the
//! app keeps its own clipboard.
use windows::core::{Error, Result};
use windows::Win32::Foundation::E_NOTIMPL;

pub fn set_remote_clipboard(_source: crate::transfer::SelectionSource) -> Result<u32> {
    Err(Error::new(
        E_NOTIMPL,
        "Remote-Dateien als virtuelle Zwischenablage sind noch nicht verfügbar",
    ))
}
