//! Real Windows access fixtures (tokens, ACLs, denial). The analytics access
//! tests and the local helper tests share them; executed only by the remote task suite.
use std::{
    io,
    mem::size_of,
    os::windows::ffi::OsStrExt,
    path::Path,
    process::Command,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_NO_TOKEN, HANDLE},
    Security::{
        CreateRestrictedToken, DuplicateTokenEx, GetFileSecurityW, GetTokenInformation,
        SecurityImpersonation, TokenImpersonation, TokenPrivileges, TokenStatistics,
        DACL_SECURITY_INFORMATION, DISABLE_MAX_PRIVILEGE, TOKEN_DUPLICATE, TOKEN_IMPERSONATE,
        TOKEN_QUERY, TOKEN_STATISTICS,
    },
    System::Threading::{
        GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken, SetThreadToken,
    },
};

pub struct Token(pub HANDLE);
impl Drop for Token {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
pub fn process_token() -> Token {
    let mut token = null_mut();
    assert_ne!(
        unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY | TOKEN_DUPLICATE,
                &mut token,
            )
        },
        0
    );
    Token(token)
}
pub fn thread_token() -> Option<Token> {
    let mut token = null_mut();
    if unsafe {
        OpenThreadToken(
            GetCurrentThread(),
            TOKEN_QUERY | TOKEN_IMPERSONATE,
            1,
            &mut token,
        )
    } != 0
    {
        Some(Token(token))
    } else {
        assert_eq!(
            io::Error::last_os_error().raw_os_error(),
            Some(ERROR_NO_TOKEN as i32)
        );
        None
    }
}
pub fn privileges(token: HANDLE) -> Vec<u8> {
    let mut needed = 0;
    unsafe {
        GetTokenInformation(token, TokenPrivileges, null_mut(), 0, &mut needed);
    }
    assert!(needed > 0);
    let mut bytes = vec![0; needed as usize];
    assert_ne!(
        unsafe {
            GetTokenInformation(
                token,
                TokenPrivileges,
                bytes.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        },
        0
    );
    bytes
}
pub fn token_id(token: HANDLE) -> (u32, i32) {
    let mut stats: TOKEN_STATISTICS = unsafe { std::mem::zeroed() };
    let mut needed = 0;
    assert_ne!(
        unsafe {
            GetTokenInformation(
                token,
                TokenStatistics,
                (&mut stats as *mut TOKEN_STATISTICS).cast(),
                size_of::<TOKEN_STATISTICS>() as u32,
                &mut needed,
            )
        },
        0
    );
    (stats.TokenId.LowPart, stats.TokenId.HighPart)
}
pub struct Identity {
    previous: Option<Token>,
    pub _token: Token,
}
impl Identity {
    pub fn new(restricted: bool) -> Self {
        let previous = thread_token();
        let process = process_token();
        let mut duplicated = null_mut();
        assert_ne!(
            unsafe {
                DuplicateTokenEx(
                    process.0,
                    TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_IMPERSONATE,
                    null(),
                    SecurityImpersonation,
                    TokenImpersonation,
                    &mut duplicated,
                )
            },
            0
        );
        let mut token = Token(duplicated);
        if restricted {
            let mut handle = null_mut();
            assert_ne!(
                unsafe {
                    CreateRestrictedToken(
                        token.0,
                        DISABLE_MAX_PRIVILEGE,
                        0,
                        null(),
                        0,
                        null(),
                        0,
                        null(),
                        &mut handle,
                    )
                },
                0
            );
            token = Token(handle);
        }
        assert_ne!(unsafe { SetThreadToken(null(), token.0) }, 0);
        Self {
            previous,
            _token: token,
        }
    }
}
impl Drop for Identity {
    fn drop(&mut self) {
        if unsafe {
            SetThreadToken(
                null(),
                self.previous.as_ref().map_or(null_mut(), |token| token.0),
            )
        } == 0
        {
            std::process::abort();
        }
    }
}
pub fn acl(path: &Path) -> Vec<u8> {
    let path: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut needed = 0;
    unsafe {
        GetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION,
            null_mut(),
            0,
            &mut needed,
        );
    }
    assert!(needed > 0);
    let mut result = vec![0; needed as usize];
    assert_ne!(
        unsafe {
            GetFileSecurityW(
                path.as_ptr(),
                DACL_SECURITY_INFORMATION,
                result.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        },
        0
    );
    result
}
pub struct DeniedDirectory(std::path::PathBuf);
impl DeniedDirectory {
    pub fn new(path: &Path) -> Self {
        let guard = Self(path.to_path_buf());
        let output = Command::new("icacls.exe")
            .arg(path)
            .args(["/deny", "*S-1-1-0:(RD)"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        guard
    }

    pub fn assert_ordinary_denied(&self) {
        // CI may start with backup privileges already enabled. Establish a
        // caller without those privileges and exhaust the lazy iterator so
        // denial may be reported either at open or on its first query.
        let _identity = Identity::new(true);
        let result =
            std::fs::read_dir(&self.0).and_then(|entries| entries.collect::<io::Result<Vec<_>>>());
        assert_eq!(
            result
                .err()
                .expect("ordinary listing must be denied")
                .kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    pub fn assert_file_read_denied(&self) {
        let _identity = Identity::new(true);
        assert_eq!(
            std::fs::File::open(&self.0)
                .err()
                .expect("ordinary data read must be denied")
                .kind(),
            io::ErrorKind::PermissionDenied
        );
    }
}
impl Drop for DeniedDirectory {
    fn drop(&mut self) {
        // Only undo the deny ACE on this owned fixture, before TempDir cleanup.
        let restored = Command::new("icacls.exe")
            .arg(&self.0)
            .args(["/remove:d", "*S-1-1-0"])
            .output()
            .is_ok_and(|output| output.status.success());
        if !restored {
            eprintln!("Fixture ACL cleanup failed: {}", self.0.display());
            if !std::thread::panicking() {
                panic!("owned fixture ACL cleanup failed");
            }
        }
    }
}
