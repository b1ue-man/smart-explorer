// Filter-aware clipboard: a COM IDataObject exposing CFSTR_FILEDESCRIPTORW +
// CFSTR_FILECONTENTS ("virtual files", the same mechanism zip folders and
// Outlook attachments use). This lets Ctrl+C carry ONLY the files that match
// the active filter — with their relative folder structure — and Explorer
// will recreate that structure on paste. A plain CF_HDROP can only ever carry
// whole folders, which is why the old Ctrl+C ignored filters.
//
// The descriptors carry relative paths ("Projekt\Unterordner\datei.txt"),
// sizes and write times (FD_WRITESTIME keeps modification dates on paste).
// File bytes are served lazily as IStreams at paste time, so copying is
// instant regardless of data volume.

#![cfg(windows)]

use std::mem::ManuallyDrop;

use windows::core::{implement, Result, PCWSTR};
use windows::Win32::Foundation::{
    BOOL, DATA_S_SAMEFORMATETC, DV_E_FORMATETC, DV_E_LINDEX, E_NOTIMPL, FILETIME,
    OLE_E_ADVISENOTSUPPORTED, S_OK,
};
use windows::Win32::System::Com::{
    IAdviseSink, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, DATADIR_GET,
    DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, STGM_READ, STGM_SHARE_DENY_NONE,
    TYMED_HGLOBAL, TYMED_ISTREAM,
};
use windows::Win32::System::DataExchange::{GetClipboardSequenceNumber, RegisterClipboardFormatW};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT,
};
use windows::Win32::System::Ole::OleSetClipboard;
use windows::Win32::UI::Shell::{
    SHCreateStdEnumFmtEtc, SHCreateStreamOnFileEx, FD_FILESIZE, FD_PROGRESSUI, FD_WRITESTIME,
    FILEDESCRIPTORW, FILEGROUPDESCRIPTORW,
};

/// One virtual file: absolute source on disk + relative path in the paste.
#[derive(Clone, Debug)]
pub struct VirtualFile {
    pub abs: String,
    /// Relative path with forward slashes; converted to backslashes for the
    /// descriptor. Explorer recreates these directories on paste.
    pub rel: String,
    pub size: u64,
    pub mtime_ms: i64,
}

fn register(name: &str) -> u16 {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    (unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) }) as u16
}

/// Unix milliseconds → Windows FILETIME (100ns ticks since 1601).
fn filetime_from_ms(ms: i64) -> FILETIME {
    const EPOCH_DIFF_MS: i64 = 11_644_473_600_000;
    let ticks = (ms + EPOCH_DIFF_MS).max(0) as u64 * 10_000;
    FILETIME {
        dwLowDateTime: (ticks & 0xFFFF_FFFF) as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    }
}

#[implement(IDataObject)]
struct VirtualFilesDataObject {
    files: Vec<VirtualFile>,
    cf_descriptor: u16,
    cf_contents: u16,
    cf_dropeffect: u16,
}

impl VirtualFilesDataObject {
    fn make_descriptor(&self) -> Result<STGMEDIUM> {
        let n = self.files.len();
        let total = std::mem::size_of::<FILEGROUPDESCRIPTORW>()
            + n.saturating_sub(1) * std::mem::size_of::<FILEDESCRIPTORW>();
        unsafe {
            let h = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, total)?;
            let ptr = GlobalLock(h) as *mut FILEGROUPDESCRIPTORW;
            if ptr.is_null() {
                return Err(windows::core::Error::from_win32());
            }
            (*ptr).cItems = n as u32;
            let fds = std::ptr::addr_of_mut!((*ptr).fgd) as *mut FILEDESCRIPTORW;
            for (i, f) in self.files.iter().enumerate() {
                // FILEGROUPDESCRIPTORW is a packed struct — build the
                // descriptor in an aligned local and write it unaligned.
                let rel = f.rel.replace('/', "\\");
                let mut name_buf = [0u16; 260];
                for (j, c) in rel.encode_utf16().take(259).enumerate() {
                    name_buf[j] = c;
                }
                let fd = FILEDESCRIPTORW {
                    dwFlags: (FD_FILESIZE.0 | FD_WRITESTIME.0 | FD_PROGRESSUI.0) as u32,
                    nFileSizeLow: (f.size & 0xFFFF_FFFF) as u32,
                    nFileSizeHigh: (f.size >> 32) as u32,
                    ftLastWriteTime: filetime_from_ms(f.mtime_ms),
                    cFileName: name_buf,
                    ..Default::default()
                };
                std::ptr::write_unaligned(fds.add(i), fd);
            }
            let _ = GlobalUnlock(h);
            Ok(STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 { hGlobal: h },
                pUnkForRelease: ManuallyDrop::new(None),
            })
        }
    }
}

impl IDataObject_Impl for VirtualFilesDataObject_Impl {
    fn GetData(&self, pformatetcin: *const FORMATETC) -> Result<STGMEDIUM> {
        unsafe {
            if pformatetcin.is_null() {
                return Err(DV_E_FORMATETC.into());
            }
            let fe = &*pformatetcin;
            let cf = fe.cfFormat;

            if cf == self.cf_descriptor && (fe.tymed & TYMED_HGLOBAL.0 as u32) != 0 {
                return self.make_descriptor();
            }

            if cf == self.cf_contents && (fe.tymed & TYMED_ISTREAM.0 as u32) != 0 {
                let i = fe.lindex;
                if i < 0 || i as usize >= self.files.len() {
                    return Err(DV_E_LINDEX.into());
                }
                let f = &self.files[i as usize];
                let path = f.abs.replace('/', "\\");
                let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
                let stream = SHCreateStreamOnFileEx(
                    PCWSTR(wide.as_ptr()),
                    STGM_READ.0 | STGM_SHARE_DENY_NONE.0,
                    0x80, // FILE_ATTRIBUTE_NORMAL
                    false,
                    None,
                )?;
                return Ok(STGMEDIUM {
                    tymed: TYMED_ISTREAM.0 as u32,
                    u: STGMEDIUM_0 {
                        pstm: ManuallyDrop::new(Some(stream)),
                    },
                    pUnkForRelease: ManuallyDrop::new(None),
                });
            }

            if cf == self.cf_dropeffect && (fe.tymed & TYMED_HGLOBAL.0 as u32) != 0 {
                let h = GlobalAlloc(GMEM_MOVEABLE, 4)?;
                let p = GlobalLock(h) as *mut u32;
                if !p.is_null() {
                    *p = 1; // DROPEFFECT_COPY
                    let _ = GlobalUnlock(h);
                }
                return Ok(STGMEDIUM {
                    tymed: TYMED_HGLOBAL.0 as u32,
                    u: STGMEDIUM_0 { hGlobal: h },
                    pUnkForRelease: ManuallyDrop::new(None),
                });
            }

            Err(DV_E_FORMATETC.into())
        }
    }

    fn GetDataHere(&self, _pformatetc: *const FORMATETC, _pmedium: *mut STGMEDIUM) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn QueryGetData(&self, pformatetc: *const FORMATETC) -> windows::core::HRESULT {
        unsafe {
            if pformatetc.is_null() {
                return DV_E_FORMATETC;
            }
            let fe = &*pformatetc;
            let cf = fe.cfFormat;
            let ok = (cf == self.cf_descriptor && (fe.tymed & TYMED_HGLOBAL.0 as u32) != 0)
                || (cf == self.cf_contents && (fe.tymed & TYMED_ISTREAM.0 as u32) != 0)
                || (cf == self.cf_dropeffect && (fe.tymed & TYMED_HGLOBAL.0 as u32) != 0);
            if ok {
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
    ) -> windows::core::HRESULT {
        unsafe {
            if !pformatetcout.is_null() {
                (*pformatetcout).ptd = std::ptr::null_mut();
            }
        }
        DATA_S_SAMEFORMATETC
    }

    fn SetData(
        &self,
        _pformatetc: *const FORMATETC,
        _pmedium: *const STGMEDIUM,
        _frelease: BOOL,
    ) -> Result<()> {
        // Explorer tries to write back "Performed DropEffect" after a paste.
        // We don't track it; refusing is harmless and standard for minimal
        // data objects.
        Err(E_NOTIMPL.into())
    }

    fn EnumFormatEtc(&self, dwdirection: u32) -> Result<IEnumFORMATETC> {
        if dwdirection == DATADIR_GET.0 as u32 {
            let fmts = [
                FORMATETC {
                    cfFormat: self.cf_descriptor,
                    ptd: std::ptr::null_mut(),
                    dwAspect: DVASPECT_CONTENT.0,
                    lindex: -1,
                    tymed: TYMED_HGLOBAL.0 as u32,
                },
                FORMATETC {
                    cfFormat: self.cf_contents,
                    ptd: std::ptr::null_mut(),
                    dwAspect: DVASPECT_CONTENT.0,
                    lindex: -1,
                    tymed: TYMED_ISTREAM.0 as u32,
                },
                FORMATETC {
                    cfFormat: self.cf_dropeffect,
                    ptd: std::ptr::null_mut(),
                    dwAspect: DVASPECT_CONTENT.0,
                    lindex: -1,
                    tymed: TYMED_HGLOBAL.0 as u32,
                },
            ];
            unsafe { SHCreateStdEnumFmtEtc(&fmts) }
        } else {
            Err(E_NOTIMPL.into())
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

/// Put the virtual files on the OLE clipboard. Must be called on the main
/// (STA, message-pumped) thread. Returns the clipboard sequence number so the
/// app can later detect whether the clipboard still holds OUR data (in-app
/// paste fast path that skips the stream round-trip).
pub fn set_clipboard(files: Vec<VirtualFile>) -> Result<u32> {
    let obj: IDataObject = VirtualFilesDataObject {
        files,
        cf_descriptor: register("FileGroupDescriptorW"),
        cf_contents: register("FileContents"),
        cf_dropeffect: register("Preferred DropEffect"),
    }
    .into();
    unsafe {
        OleSetClipboard(&obj)?;
        Ok(GetClipboardSequenceNumber())
    }
}

/// Current clipboard sequence number (for in-app fast-path detection).
pub fn clipboard_sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}
