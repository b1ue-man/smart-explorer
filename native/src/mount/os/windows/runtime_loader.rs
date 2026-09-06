use std::{
    ffi::OsString,
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    ptr::null_mut,
};

use windows_sys::Win32::{
    Foundation::{FreeLibrary, GetLastError, ERROR_BAD_EXE_FORMAT, ERROR_MOD_NOT_FOUND, HMODULE},
    System::LibraryLoader::{
        GetModuleFileNameW, GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
    },
};

use super::{private_payload::PrivatePayload, runtime::DokanyPreflightError};

pub(super) struct LoadedModule {
    handle: HMODULE,
    _payload: Option<PrivatePayload>,
}

impl LoadedModule {
    pub(super) fn system32() -> Result<Self, DokanyPreflightError> {
        let name = "dokan2.dll\0".encode_utf16().collect::<Vec<_>>();
        let handle = unsafe {
            LoadLibraryExW(name.as_ptr(), null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32)
        };
        if handle.is_null() {
            return Err(load_error());
        }
        Ok(Self { handle, _payload: None })
    }

    pub(super) fn private(payload: PrivatePayload) -> Result<Self, DokanyPreflightError> {
        payload.validate_directories().map_err(rejected)?;
        let mut path = payload.path.as_os_str().encode_wide()
            .map(|unit| if unit == b'/' as u16 { b'\\' as u16 } else { unit })
            .collect::<Vec<_>>();
        if path.contains(&0) {
            return Err(rejected(io::Error::other("private DLL path contains NUL")));
        }
        path.push(0);
        // Absolute target only; dependencies resolve solely from System32.
        let handle = unsafe {
            LoadLibraryExW(path.as_ptr(), null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32)
        };
        if handle.is_null() {
            return Err(rejected(io::Error::last_os_error()));
        }
        let module = Self { handle, _payload: Some(payload) };
        let actual = module.path().map_err(rejected)?;
        if let Some(payload) = module._payload.as_ref() {
            payload.verify_loaded_path(&actual).map_err(rejected)?;
        }
        Ok(module)
    }

    pub(super) fn path(&self) -> io::Result<PathBuf> {
        let mut path = vec![0u16; 32_768];
        let length = unsafe { GetModuleFileNameW(self.handle, path.as_mut_ptr(), path.len() as u32) };
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        if length as usize >= path.len() {
            return Err(io::Error::other("loaded DLL pathname exceeds Windows limit"));
        }
        Ok(PathBuf::from(OsString::from_wide(&path[..length as usize])))
    }

    pub(super) fn symbol(
        &self,
        name: &'static [u8],
        display_name: &'static str,
    ) -> Result<unsafe extern "system" fn() -> isize, DokanyPreflightError> {
        unsafe { GetProcAddress(self.handle, name.as_ptr()) }.ok_or(
            DokanyPreflightError::RuntimeSymbolMissing { symbol: display_name },
        )
    }
}

impl Drop for LoadedModule {
    fn drop(&mut self) {
        // Payload ownership denies file writes/deletion until this returns.
        unsafe { FreeLibrary(self.handle) };
    }
}

fn load_error() -> DokanyPreflightError {
    let win32_error = unsafe { GetLastError() };
    match win32_error {
        ERROR_MOD_NOT_FOUND => DokanyPreflightError::RuntimeNotInstalled,
        ERROR_BAD_EXE_FORMAT => DokanyPreflightError::RuntimeArchitectureMismatch,
        _ => DokanyPreflightError::RuntimeLoadFailed { win32_error },
    }
}

fn rejected(error: io::Error) -> DokanyPreflightError {
    DokanyPreflightError::PrivateRuntimeRejected { detail: error.to_string() }
}
