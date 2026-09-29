//! The IDataObject Explorer pastes from or receives by drag: the file list
//! (FILEGROUPDESCRIPTORW, listed on the first request), one FILECONTENTS
//! stream per file and the preferred copy effect. IDataObjectAsyncCapability
//! lets Explorer copy in the background instead of blocking its window.
use super::catalog::render;
use super::handoff::Handoff;
use super::stream::RemoteStream;
use super::worker::LifeToken;
use std::mem::ManuallyDrop;
use std::sync::Arc;
use windows::core::{implement, Error, Result, HRESULT};
use windows::Win32::Foundation::{
    GlobalFree, BOOL, DATA_S_SAMEFORMATETC, DV_E_FORMATETC, DV_E_LINDEX, E_NOTIMPL, E_UNEXPECTED,
    HGLOBAL, OLE_E_ADVISENOTSUPPORTED, STG_E_MEDIUMFULL, S_OK,
};
use windows::Win32::System::Com::{
    IAdviseSink, IBindCtx, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, IStream,
    DATADIR_GET, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL, TYMED_ISTREAM,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::DROPEFFECT_COPY;
use windows::Win32::UI::Shell::{
    IDataObjectAsyncCapability, IDataObjectAsyncCapability_Impl, SHCreateStdEnumFmtEtc,
};

/// Clipboard format ids of the offered formats.
#[derive(Clone, Copy)]
pub(super) struct Formats {
    pub(super) descriptor: u16,
    pub(super) contents: u16,
    pub(super) drop_effect: u16,
}

impl Formats {
    pub(super) fn register() -> Self {
        use crate::virtual_clipboard::imp::register;
        Self {
            descriptor: register("FileGroupDescriptorW"),
            contents: register("FileContents"),
            drop_effect: register("Preferred DropEffect"),
        }
    }

    fn offered(&self, format: &FORMATETC) -> bool {
        let hglobal = (format.tymed & TYMED_HGLOBAL.0 as u32) != 0;
        let istream = (format.tymed & TYMED_ISTREAM.0 as u32) != 0;
        (format.cfFormat == self.descriptor && hglobal)
            || (format.cfFormat == self.contents && istream)
            || (format.cfFormat == self.drop_effect && hglobal)
    }
}

#[implement(IDataObject, IDataObjectAsyncCapability)]
pub(super) struct RemoteDataObject {
    handoff: Arc<Handoff>,
    formats: Formats,
    life: Arc<LifeToken>,
}

impl RemoteDataObject {
    pub(super) fn new(handoff: Arc<Handoff>, life: Arc<LifeToken>) -> Self {
        Self {
            handoff,
            formats: Formats::register(),
            life,
        }
    }

    fn descriptor(&self) -> Result<STGMEDIUM> {
        self.handoff.begin_paste();
        let catalog = self.handoff.catalog()?;
        if !catalog.complete {
            // Never hand Explorer a partial list.
            let reason = catalog.problems.first().map(|(_, reason)| reason.as_str());
            return Err(Error::new(
                E_UNEXPECTED,
                reason.unwrap_or("Die Übergabe wurde beendet"),
            ));
        }
        match render(&catalog.entries, self.handoff.alloc) {
            Ok(handle) => Ok(hglobal_medium(handle)),
            Err(error) => {
                if error.code() == STG_E_MEDIUMFULL {
                    self.handoff.sessions.too_large();
                }
                Err(error)
            }
        }
    }

    fn contents(&self, lindex: i32) -> Result<STGMEDIUM> {
        let catalog = self.handoff.catalog()?;
        // lindex is the zero-based index into the descriptor's entries.
        let index = usize::try_from(lindex)
            .ok()
            .filter(|index| *index < catalog.entries.len())
            .ok_or_else(|| Error::from(DV_E_LINDEX))?;
        let stream: IStream =
            RemoteStream::open(&self.handoff, &catalog, index, self.life.clone())?.into();
        Ok(STGMEDIUM {
            tymed: TYMED_ISTREAM.0 as u32,
            u: STGMEDIUM_0 {
                pstm: ManuallyDrop::new(Some(stream)),
            },
            pUnkForRelease: ManuallyDrop::new(None),
        })
    }
}

fn hglobal_medium(handle: HGLOBAL) -> STGMEDIUM {
    // pUnkForRelease = None: the receiver owns and frees the block.
    STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 { hGlobal: handle },
        pUnkForRelease: ManuallyDrop::new(None),
    }
}

/// Remote files can only be copied; moving them out is not offered.
fn drop_effect_medium() -> Result<STGMEDIUM> {
    let effect = DROPEFFECT_COPY.0;
    unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE, std::mem::size_of_val(&effect))?;
        let target = GlobalLock(handle).cast::<u32>();
        if target.is_null() {
            let error = Error::from_win32();
            let _ = GlobalFree(handle);
            return Err(error);
        }
        target.write_unaligned(effect);
        let _ = GlobalUnlock(handle);
        Ok(hglobal_medium(handle))
    }
}

fn format_etc(format: u16, tymed: u32) -> FORMATETC {
    FORMATETC {
        cfFormat: format,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed,
    }
}

impl IDataObject_Impl for RemoteDataObject_Impl {
    fn GetData(&self, pformatetcin: *const FORMATETC) -> Result<STGMEDIUM> {
        let Some(format) = (unsafe { pformatetcin.as_ref() }) else {
            return Err(DV_E_FORMATETC.into());
        };
        if !self.formats.offered(format) {
            return Err(DV_E_FORMATETC.into());
        }
        if format.cfFormat == self.formats.descriptor {
            self.descriptor()
        } else if format.cfFormat == self.formats.contents {
            self.contents(format.lindex)
        } else {
            drop_effect_medium()
        }
    }

    fn GetDataHere(&self, _pformatetc: *const FORMATETC, _pmedium: *mut STGMEDIUM) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn QueryGetData(&self, pformatetc: *const FORMATETC) -> HRESULT {
        match unsafe { pformatetc.as_ref() } {
            Some(format) if self.formats.offered(format) => S_OK,
            _ => DV_E_FORMATETC,
        }
    }

    fn GetCanonicalFormatEtc(
        &self,
        _pformatectin: *const FORMATETC,
        pformatetcout: *mut FORMATETC,
    ) -> HRESULT {
        if let Some(out) = unsafe { pformatetcout.as_mut() } {
            out.ptd = std::ptr::null_mut();
        }
        DATA_S_SAMEFORMATETC
    }

    fn SetData(
        &self,
        _pformatetc: *const FORMATETC,
        _pmedium: *const STGMEDIUM,
        _frelease: BOOL,
    ) -> Result<()> {
        // Performed/paste-succeeded effects only matter for moves, which a
        // remote source does not offer.
        Err(E_NOTIMPL.into())
    }

    fn EnumFormatEtc(&self, dwdirection: u32) -> Result<IEnumFORMATETC> {
        if dwdirection != DATADIR_GET.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        let formats = [
            format_etc(self.formats.descriptor, TYMED_HGLOBAL.0 as u32),
            format_etc(self.formats.contents, TYMED_ISTREAM.0 as u32),
            format_etc(self.formats.drop_effect, TYMED_HGLOBAL.0 as u32),
        ];
        unsafe { SHCreateStdEnumFmtEtc(&formats) }
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

impl IDataObjectAsyncCapability_Impl for RemoteDataObject_Impl {
    fn SetAsyncMode(&self, fdoopasync: BOOL) -> Result<()> {
        self.handoff.set_async(fdoopasync.as_bool());
        Ok(())
    }

    fn GetAsyncMode(&self) -> Result<BOOL> {
        Ok(self.handoff.is_async().into())
    }

    fn StartOperation(&self, _pbcreserved: Option<&IBindCtx>) -> Result<()> {
        self.handoff.operation_started();
        Ok(())
    }

    fn InOperation(&self) -> Result<BOOL> {
        Ok(self.handoff.in_operation().into())
    }

    fn EndOperation(
        &self,
        _hresult: HRESULT,
        _pbcreserved: Option<&IBindCtx>,
        _dweffects: u32,
    ) -> Result<()> {
        self.handoff.operation_ended();
        Ok(())
    }
}
