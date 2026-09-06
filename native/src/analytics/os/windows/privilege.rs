//! Backup read access on a private, thread-scoped token. Never adjust the process token.
use std::{
    io,
    marker::PhantomData,
    ptr::{null, null_mut},
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, SetLastError, ERROR_NO_TOKEN, HANDLE, LUID},
    Security::{
        AdjustTokenPrivileges, DuplicateTokenEx, LookupPrivilegeValueW, SecurityImpersonation,
        TokenImpersonation, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES,
        TOKEN_DUPLICATE, TOKEN_IMPERSONATE, TOKEN_PRIVILEGES, TOKEN_QUERY,
    },
    System::Threading::{
        GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken, SetThreadToken,
    },
};

struct Token(HANDLE);
impl Drop for Token {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub(super) struct BackupRead {
    previous: Option<Token>,
    _token: Token,
    // A thread identity must never migrate through Send/Sync.
    _thread_only: PhantomData<Rc<()>>,
}

impl BackupRead {
    pub(super) fn enable() -> io::Result<Self> {
        let mut handle = null_mut();
        let previous = if unsafe {
            OpenThreadToken(
                GetCurrentThread(),
                TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_IMPERSONATE,
                1,
                &mut handle,
            )
        } != 0
        {
            Some(Token(handle))
        } else {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_NO_TOKEN as i32) {
                return Err(error);
            }
            None
        };
        let process;
        let source = if let Some(token) = &previous {
            token.0
        } else {
            if unsafe {
                OpenProcessToken(
                    GetCurrentProcess(),
                    TOKEN_QUERY | TOKEN_DUPLICATE,
                    &mut handle,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            process = Token(handle);
            process.0
        };
        let mut duplicated = null_mut();
        if unsafe {
            DuplicateTokenEx(
                source,
                TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES | TOKEN_IMPERSONATE,
                null(),
                SecurityImpersonation,
                TokenImpersonation,
                &mut duplicated,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let token = Token(duplicated);
        let name: Vec<u16> = "SeBackupPrivilege\0".encode_utf16().collect();
        let mut luid = LUID {
            LowPart: 0,
            HighPart: 0,
        };
        if unsafe { LookupPrivilegeValueW(null(), name.as_ptr(), &mut luid) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let state = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        unsafe {
            SetLastError(0);
        }
        let ok = unsafe { AdjustTokenPrivileges(token.0, 0, &state, 0, null_mut(), null_mut()) };
        let error = unsafe { GetLastError() };
        // A true return can still mean ERROR_NOT_ALL_ASSIGNED.
        if ok == 0 || error != 0 {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        if unsafe { SetThreadToken(null(), token.0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            previous,
            _token: token,
            _thread_only: PhantomData,
        })
    }
}

impl Drop for BackupRead {
    fn drop(&mut self) {
        let previous = self.previous.as_ref().map_or(null_mut(), |token| token.0);
        if unsafe { SetThreadToken(null(), previous) } == 0 {
            // Microsoft requires shutdown if impersonation cannot be reverted:
            // continuing would run unrelated work with an unexpected identity.
            std::process::abort();
        }
    }
}
