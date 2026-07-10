//! Native drag-OUT to Explorer (#6c). When an internal file drag leaves the
//! window, the app hands the selected files to the OS via OLE `DoDragDrop` with
//! a minimal CF_HDROP data object, so they can be dropped onto Explorer, the
//! desktop, or any app. Self-contained and best-effort: every COM call is
//! wrapped so a failure just aborts the out-drag (no panic) and the in-app drag
//! behaviour is unaffected. OLE is already initialised on this (main) thread by
//! `shell_menu::init_com()`.
#![cfg(windows)]

use std::cell::RefCell;
use std::mem::ManuallyDrop;
use windows::core::{implement, Error, Result, HRESULT};
use windows::Win32::Foundation::{
    BOOL, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, DV_E_FORMATETC,
    E_INVALIDARG, OLE_E_ADVISENOTSUPPORTED, POINT, S_OK,
};
use windows::Win32::System::Com::{
    IAdviseSink, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, DATADIR_GET,
    FORMATETC, STGMEDIUM, TYMED_HGLOBAL,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, CF_HDROP, DROPEFFECT, DROPEFFECT_COPY,
    DROPEFFECT_MOVE,
};
use windows::Win32::System::SystemServices::MK_LBUTTON;
use windows::Win32::UI::Shell::{SHCreateStdEnumFmtEtc, DROPFILES};

use super::{DragOutEffect, DragOutOutcome};

// ─── CF_HDROP global build ───────────────────────────────────────────────────

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Build a global memory block holding a DROPFILES + double-null-terminated,
/// wide file list — the payload Explorer expects for CF_HDROP.
unsafe fn build_hdrop(files: &[String]) -> Result<windows::Win32::Foundation::HGLOBAL> {
    // wide list: path1\0 path2\0 … \0
    let mut list: Vec<u16> = Vec::new();
    for f in files {
        // Explorer wants backslash paths.
        list.extend(to_wide(&f.replace('/', "\\")));
        list.push(0);
    }
    list.push(0);

    let header = std::mem::size_of::<DROPFILES>();
    let bytes = header + list.len() * 2;
    let hglobal = GlobalAlloc(GMEM_MOVEABLE, bytes)?;
    let base = GlobalLock(hglobal) as *mut u8;
    if base.is_null() {
        return Err(windows::core::Error::from_win32());
    }
    // DROPFILES header
    let df = base as *mut DROPFILES;
    (*df).pFiles = header as u32;
    (*df).pt = POINT { x: 0, y: 0 };
    (*df).fNC = BOOL(0);
    (*df).fWide = BOOL(1);
    // wide file list right after the header
    let dst = base.add(header) as *mut u16;
    std::ptr::copy_nonoverlapping(list.as_ptr(), dst, list.len());
    GlobalUnlock(hglobal).ok();
    Ok(hglobal)
}

fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: 1, // DVASPECT_CONTENT
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

// ─── Minimal IDataObject (CF_HDROP only) ─────────────────────────────────────

#[implement(IDataObject)]
struct FileData {
    files: Vec<String>,
    fmt: RefCell<Vec<FORMATETC>>,
}

impl IDataObject_Impl for FileData_Impl {
    fn GetData(&self, pformatetcin: *const FORMATETC) -> Result<STGMEDIUM> {
        unsafe {
            let req = &*pformatetcin;
            if req.cfFormat != CF_HDROP.0 || (req.tymed & TYMED_HGLOBAL.0 as u32) == 0 {
                return Err(DV_E_FORMATETC.into());
            }
            let hglobal = build_hdrop(&self.files)?;
            Ok(STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: windows::Win32::System::Com::STGMEDIUM_0 { hGlobal: hglobal },
                pUnkForRelease: ManuallyDrop::new(None),
            })
        }
    }

    fn GetDataHere(&self, _pformatetc: *const FORMATETC, _pmedium: *mut STGMEDIUM) -> Result<()> {
        Err(DV_E_FORMATETC.into())
    }

    fn QueryGetData(&self, pformatetc: *const FORMATETC) -> HRESULT {
        unsafe {
            let req = &*pformatetc;
            if req.cfFormat == CF_HDROP.0 && (req.tymed & TYMED_HGLOBAL.0 as u32) != 0 {
                S_OK
            } else {
                DV_E_FORMATETC
            }
        }
    }

    fn GetCanonicalFormatEtc(
        &self,
        _pformatectin: *const FORMATETC,
        pformatetcout: *mut FORMATETC,
    ) -> HRESULT {
        unsafe {
            if !pformatetcout.is_null() {
                (*pformatetcout).ptd = std::ptr::null_mut();
            }
        }
        // E_NOTIMPL semantics: caller should use the format as-is.
        windows::Win32::Foundation::E_NOTIMPL
    }

    fn SetData(
        &self,
        _pformatetc: *const FORMATETC,
        _pmedium: *const STGMEDIUM,
        _frelease: BOOL,
    ) -> Result<()> {
        Err(DV_E_FORMATETC.into())
    }

    fn EnumFormatEtc(&self, dwdirection: u32) -> Result<IEnumFORMATETC> {
        if dwdirection == DATADIR_GET.0 as u32 {
            unsafe { SHCreateStdEnumFmtEtc(&self.fmt.borrow()) }
        } else {
            Err(windows::Win32::Foundation::E_NOTIMPL.into())
        }
    }

    fn DAdvise(
        &self,
        _pformatetc: *const FORMATETC,
        _advf: u32,
        _padvsink: Option<&IAdviseSink>,
    ) -> Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn DUnadvise(&self, _dwconnection: u32) -> Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumDAdvise(&self) -> Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

// ─── Minimal IDropSource ─────────────────────────────────────────────────────

#[implement(IDropSource)]
struct DropSource;

impl IDropSource_Impl for DropSource_Impl {
    fn QueryContinueDrag(
        &self,
        fescapepressed: BOOL,
        grfkeystate: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
    ) -> HRESULT {
        if fescapepressed.as_bool() {
            return DRAGDROP_S_CANCEL;
        }
        // Left button released → commit the drop.
        if (grfkeystate.0 & MK_LBUTTON.0) == 0 {
            return DRAGDROP_S_DROP;
        }
        S_OK
    }

    fn GiveFeedback(&self, _dweffect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

// ─── Public entry point ──────────────────────────────────────────────────────

fn classify_drag_result(hr: HRESULT, effect: DROPEFFECT) -> Result<DragOutOutcome> {
    if hr == DRAGDROP_S_CANCEL {
        return Ok(DragOutOutcome::Cancelled);
    }
    if hr.is_err() {
        return Err(hr.into());
    }

    // DoDragDrop documents both S_OK and DRAGDROP_S_DROP as successful return
    // values. The target's output effect is authoritative after either one.
    let effect = if (effect & DROPEFFECT_MOVE) == DROPEFFECT_MOVE {
        DragOutEffect::Move
    } else if (effect & DROPEFFECT_COPY) == DROPEFFECT_COPY {
        DragOutEffect::Copy
    } else {
        DragOutEffect::None
    };
    Ok(DragOutOutcome::Dropped(effect))
}

/// Run an OS drag for `files`, blocking until the user drops or cancels.
/// Cancellation and an accepted drop are returned as explicit outcomes; COM
/// and OLE failures retain their HRESULT in the returned error.
pub fn drag_out(files: &[String]) -> Result<DragOutOutcome> {
    let files: Vec<String> = files.iter().filter(|p| !p.is_empty()).cloned().collect();
    if files.is_empty() {
        return Err(Error::from_hresult(E_INVALIDARG));
    }
    let data: IDataObject = FileData {
        files,
        fmt: RefCell::new(vec![hdrop_format()]),
    }
    .into();
    let source: IDropSource = DropSource.into();
    let mut effect = DROPEFFECT::default();
    let hr = unsafe {
        DoDragDrop(
            &data,
            &source,
            DROPEFFECT_COPY | DROPEFFECT_MOVE,
            &mut effect,
        )
    };
    classify_drag_result(hr, effect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::E_UNEXPECTED;

    #[test]
    fn classifies_cancel_without_reading_an_effect() {
        assert_eq!(
            classify_drag_result(DRAGDROP_S_CANCEL, DROPEFFECT_MOVE).unwrap(),
            DragOutOutcome::Cancelled
        );
    }

    #[test]
    fn classifies_successful_copy_move_and_no_effect() {
        assert_eq!(
            classify_drag_result(DRAGDROP_S_DROP, DROPEFFECT_COPY).unwrap(),
            DragOutOutcome::Dropped(DragOutEffect::Copy)
        );
        assert_eq!(
            classify_drag_result(S_OK, DROPEFFECT_MOVE).unwrap(),
            DragOutOutcome::Dropped(DragOutEffect::Move)
        );
        assert_eq!(
            classify_drag_result(DRAGDROP_S_DROP, DROPEFFECT::default()).unwrap(),
            DragOutOutcome::Dropped(DragOutEffect::None)
        );
    }

    #[test]
    fn preserves_failure_hresult() {
        let error = classify_drag_result(E_UNEXPECTED, DROPEFFECT::default()).unwrap_err();
        assert_eq!(error.code(), E_UNEXPECTED);
    }

    #[test]
    fn rejects_an_empty_file_list_before_entering_ole() {
        let error = drag_out(&[]).unwrap_err();
        assert_eq!(error.code(), E_INVALIDARG);
    }
}
