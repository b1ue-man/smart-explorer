//! `IExplorerCommand` COM handler for the Windows 11 **modern** (main) context
//! menu — the "In Smart Explorer öffnen" verb that appears WITHOUT going through
//! "Show more options".
//!
//! A legacy registry verb (which the app already installs via `shell_register`)
//! only ever shows under "Show more options" on Win11. The main menu requires
//! this `IExplorerCommand` COM handler **declared by a packaged identity**
//! (sparse MSIX). The DLL here is the feasible half; the wall is **signing the
//! package with a cert the machine trusts** (see docs/WIN11_CONTEXT_MENU.md and
//! docs/GOTCHAS.md). Built + compile-verified for x86_64-pc-windows-gnu;
//! shippable once signed.
//!
//! This is an in-proc COM server: `DllGetClassObject` hands out an
//! `IClassFactory` for `CLSID_OPEN`, which creates the `OpenCommand`. The host
//! exe is resolved as a sibling of this DLL (both live in the package).
#![cfg(windows)]
#![allow(non_snake_case)]

use core::ffi::c_void;
use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};
use windows::core::{implement, w, IUnknown, Interface, Result, GUID, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    BOOL, CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_NOTIMPL, E_POINTER, HINSTANCE,
    HMODULE, MAX_PATH, S_FALSE, S_OK,
};
use windows::Win32::System::Com::{CoTaskMemFree, IBindCtx, IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::UI::Shell::{
    IEnumExplorerCommand, IExplorerCommand, IExplorerCommand_Impl, IShellItemArray, SHStrDupW,
    ShellExecuteW, ECF_DEFAULT, ECS_ENABLED, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// CLSID of the handler. The sparse package manifest references this exact GUID.
/// {7F3B1E20-9C4A-4D8E-A1B2-3C4D5E6F7081}
const CLSID_OPEN: GUID = GUID::from_u128(0x7F3B1E20_9C4A_4D8E_A1B2_3C4D5E6F7081);

static MODULE: AtomicIsize = AtomicIsize::new(0);
static LOCKS: AtomicI32 = AtomicI32::new(0);

const DLL_PROCESS_ATTACH: u32 = 1;

#[no_mangle]
extern "system" fn DllMain(hinst: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        MODULE.store(hinst.0 as isize, Ordering::Relaxed);
    }
    BOOL(1)
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Smart Explorer.exe, resolved as a sibling of this DLL (both in the package).
fn exe_path() -> Option<String> {
    let hmod = HMODULE(MODULE.load(Ordering::Relaxed) as *mut c_void);
    let mut buf = [0u16; MAX_PATH as usize];
    let len = unsafe { GetModuleFileNameW(hmod, &mut buf) } as usize;
    if len == 0 {
        return None;
    }
    let dll = String::from_utf16_lossy(&buf[..len]);
    let (dir, _) = dll.rsplit_once('\\')?;
    Some(format!("{dir}\\Smart Explorer.exe"))
}

#[implement(IExplorerCommand)]
struct OpenCommand;

impl IExplorerCommand_Impl for OpenCommand_Impl {
    fn GetTitle(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        unsafe { SHStrDupW(w!("In Smart Explorer öffnen")) }
    }
    fn GetIcon(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }
    fn GetToolTip(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }
    fn GetCanonicalName(&self) -> Result<GUID> {
        Ok(GUID::zeroed())
    }
    fn GetState(&self, _items: Option<&IShellItemArray>, _slow: BOOL) -> Result<u32> {
        Ok(ECS_ENABLED.0 as u32)
    }
    fn Invoke(&self, items: Option<&IShellItemArray>, _pbc: Option<&IBindCtx>) -> Result<()> {
        let exe = exe_path().ok_or_else(|| windows::core::Error::from(E_POINTER))?;
        let exe_w = to_wide(&exe);
        if let Some(arr) = items {
            unsafe {
                let n = arr.GetCount().unwrap_or(0);
                for i in 0..n {
                    if let Ok(item) = arr.GetItemAt(i) {
                        if let Ok(path) = item.GetDisplayName(SIGDN_FILESYSPATH) {
                            ShellExecuteW(
                                None,
                                w!("open"),
                                PCWSTR(exe_w.as_ptr()),
                                PCWSTR(path.0 as *const u16),
                                PCWSTR::null(),
                                SW_SHOWNORMAL,
                            );
                            CoTaskMemFree(Some(path.0 as *const c_void));
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_DEFAULT.0 as u32)
    }
    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Option<&IUnknown>,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> Result<()> {
        unsafe {
            if !ppv.is_null() {
                *ppv = core::ptr::null_mut();
            }
        }
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let cmd: IExplorerCommand = OpenCommand.into();
        unsafe { cmd.query(riid, ppv).ok() }
    }
    fn LockServer(&self, lock: BOOL) -> Result<()> {
        if lock.as_bool() {
            LOCKS.fetch_add(1, Ordering::Relaxed);
        } else {
            LOCKS.fetch_sub(1, Ordering::Relaxed);
        }
        Ok(())
    }
}

#[no_mangle]
extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    unsafe {
        if ppv.is_null() {
            return E_POINTER;
        }
        *ppv = core::ptr::null_mut();
        if rclsid.is_null() || *rclsid != CLSID_OPEN {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory.into();
        factory.query(riid, ppv)
    }
}

#[no_mangle]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    if LOCKS.load(Ordering::Relaxed) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}
