//! Dragging a remote selection out to Explorer as virtual files: nothing is
//! downloaded first. The data object lives on its own thread (listing and
//! reads never block this GUI thread); DoDragDrop gets a proxy to it, and
//! with IDataObjectAsyncCapability Explorer keeps copying after the drop.
use super::imp::{classify_drag_result, DropSource};
use super::DragOutOutcome;
use windows::core::{Error, Result};
use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::System::Ole::{DoDragDrop, IDropSource, DROPEFFECT, DROPEFFECT_COPY};

/// Runs an OS drag of `source`, blocking until the user drops or cancels.
/// Remote files can only be copied out; moving them is not offered.
pub fn drag_out_remote(source: crate::transfer::SelectionSource) -> Result<DragOutOutcome> {
    if source.paths.is_empty() {
        return Err(Error::from_hresult(E_INVALIDARG));
    }
    let mut drag = crate::virtual_clipboard::start_remote_drag(source)?;
    let data = drag.data_object()?;
    let drop_source: IDropSource = DropSource.into();
    let mut effect = DROPEFFECT::default();
    let hr = unsafe { DoDragDrop(&data, &drop_source, DROPEFFECT_COPY, &mut effect) };
    // Only this thread's proxy goes; Explorer holds its own reference while
    // it copies in the background.
    drop(data);
    drop(drag);
    classify_drag_result(hr, effect)
}
