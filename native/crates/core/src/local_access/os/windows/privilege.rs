//! Backup read access on a private, thread-scoped token. Never adjust the process token.
use std::{
    io,
    marker::PhantomData,
    ptr::{null, null_mut},
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
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

/// The process token was found without `SeBackupPrivilege` (it cannot gain
/// it later); until the next scan begins, no further attempt is made for a
/// thread without its own token. A thread token is always tried.
static PROCESS_LACKS_BACKUP: AtomicBool = AtomicBool::new(false);
const ERROR_NOT_ALL_ASSIGNED: u32 = 1300;

/// A scan begins: the backup privilege is tried once again.
pub fn begin_scan() {
    PROCESS_LACKS_BACKUP.store(false, Ordering::Relaxed);
}

struct Token(HANDLE);
impl Drop for Token {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn parallel_scan_allowed() -> bool {
    let mut handle = null_mut();
    if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut handle) } != 0 {
        let _token = Token(handle);
        return false;
    }
    // New worker threads do not inherit impersonation. Only a positively
    // established absence of a thread token permits leaving this thread.
    io::Error::last_os_error().raw_os_error() == Some(ERROR_NO_TOKEN as i32)
}

pub struct BackupRead {
    previous: Option<Token>,
    _token: Token,
    // A thread identity must never migrate through Send/Sync.
    _thread_only: PhantomData<Rc<()>>,
}

impl BackupRead {
    pub fn enable() -> io::Result<Self> {
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
        if previous.is_none() && PROCESS_LACKS_BACKUP.load(Ordering::Relaxed) {
            return Err(io::Error::from_raw_os_error(ERROR_NOT_ALL_ASSIGNED as i32));
        }
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
            if previous.is_none() && error == ERROR_NOT_ALL_ASSIGNED {
                PROCESS_LACKS_BACKUP.store(true, Ordering::Relaxed);
            }
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
