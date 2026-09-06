//! Real Windows access fixtures, executed only by the single remote task suite.
use super::{directory, privilege::BackupRead};
use crate::analytics::{scan, Progress, ScanStatus};
use std::{
    fs::OpenOptions,
    io,
    mem::size_of,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::Path,
    process::Command,
    ptr::{null, null_mut},
    sync::atomic::Ordering,
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

struct Token(HANDLE);
impl Drop for Token {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn process_token() -> Token {
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
fn thread_token() -> Option<Token> {
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
fn privileges(token: HANDLE) -> Vec<u8> {
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
fn token_id(token: HANDLE) -> (u32, i32) {
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
struct Identity {
    previous: Option<Token>,
    _token: Token,
}
impl Identity {
    fn new(restricted: bool) -> Self {
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
fn acl(path: &Path) -> Vec<u8> {
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
struct DeniedDirectory(std::path::PathBuf);
impl DeniedDirectory {
    fn new(path: &Path) -> Self {
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
}
impl Drop for DeniedDirectory {
    fn drop(&mut self) {
        // Only undo the deny ACE on this owned fixture, before TempDir cleanup.
        let restored = Command::new("icacls.exe")
            .arg(&self.0)
            .args(["/remove:d", "*S-1-1-0"])
            .status()
            .is_ok_and(|status| status.success());
        if !restored {
            eprintln!("Fixture ACL cleanup failed: {}", self.0.display());
            if !std::thread::panicking() {
                panic!("owned fixture ACL cleanup failed");
            }
        }
    }
}
fn remote_only() {
    assert_eq!(
        std::env::var("SMART_EXPLORER_ANALYTICS_TASK").as_deref(),
        Ok("1"),
        "Use the checked-in remote analytics task entrypoint"
    );
}

#[test]
#[ignore = "real ACL and token fixture: remote Windows task only"]
fn analytics_access_task_real_denied_directory_locked_files_and_unchanged_acl() {
    remote_only();
    let process = process_token();
    let before = privileges(process.0);
    let previous = thread_token().map(|token| token_id(token.0));
    {
        let _backup = BackupRead::enable().expect("remote runner must own SeBackupPrivilege");
    }
    let fixture = tempfile::tempdir().unwrap();
    let protected = fixture.path().join("protected");
    std::fs::create_dir_all(protected.join("nested")).unwrap();
    std::fs::write(protected.join("locked.bin"), vec![0; 37]).unwrap();
    std::fs::write(protected.join("nested/note.md"), vec![0; 13]).unwrap();
    std::fs::write(fixture.path().join("sibling"), vec![0; 7]).unwrap();
    let _locked = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(protected.join("locked.bin"))
        .unwrap();
    assert!(std::fs::File::open(protected.join("locked.bin")).is_err());
    let _denied = DeniedDirectory::new(&protected);
    assert_eq!(
        std::fs::read_dir(&protected).err().unwrap().kind(),
        io::ErrorKind::PermissionDenied,
        "ordinary enumeration must reproduce the reported access failure"
    );
    let before_acl = acl(&protected);
    let progress = Progress::default();
    let outcome = scan(fixture.path(), &progress);
    assert_eq!(outcome.status, ScanStatus::Complete, "{:?}", outcome.issues);
    assert_eq!(outcome.tree.unwrap().size, 57);
    assert_eq!(progress.files.load(Ordering::Relaxed), 3);
    assert_eq!(outcome.permission_denied, 0);
    assert_eq!(
        acl(&protected),
        before_acl,
        "scanner must never rewrite ACLs"
    );
    assert_eq!(privileges(process.0), before, "process privileges changed");
    assert_eq!(thread_token().map(|token| token_id(token.0)), previous);
    // The root-denied route must also work, not only child-directory retries.
    let outcome = scan(&protected, &Progress::default());
    assert_eq!(outcome.status, ScanStatus::Complete, "{:?}", outcome.issues);
    assert_eq!(outcome.tree.unwrap().size, 50);
}

#[test]
#[ignore = "thread impersonation fixture: remote Windows task only"]
fn analytics_access_task_restricted_identity_never_falls_back_to_process_authority() {
    remote_only();
    let fixture = tempfile::tempdir().unwrap();
    let protected = fixture.path().join("protected");
    std::fs::create_dir(&protected).unwrap();
    std::fs::write(protected.join("file"), "secret").unwrap();
    let _denied = DeniedDirectory::new(&protected);
    {
        let identity = Identity::new(false);
        let previous_id = token_id(identity._token.0);
        {
            let _backup = BackupRead::enable().unwrap();
        }
        assert_eq!(token_id(thread_token().unwrap().0), previous_id);
    }
    {
        let identity = Identity::new(true);
        let previous_id = token_id(identity._token.0);
        assert!(BackupRead::enable().is_err());
        let error = directory::read_directory(&protected)
            .err()
            .expect("restricted token must not bypass denial");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(token_id(thread_token().unwrap().0), previous_id);
    }
}

#[test]
#[ignore = "real junction fixture: remote Windows task only"]
fn analytics_access_task_redirect_children_are_traversal_boundaries() {
    remote_only();
    let fixture = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("outside.bin"), vec![0; 100]).unwrap();
    std::fs::write(fixture.path().join("inside.bin"), vec![0; 7]).unwrap();
    let link = fixture.path().join("junction");
    let created = Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&link)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let outcome = scan(fixture.path(), &Progress::default());
    // Remove the junction itself, never recursively traverse its target.
    std::fs::remove_dir(&link).unwrap();
    assert_eq!(outcome.status, ScanStatus::Complete, "{:?}", outcome.issues);
    assert_eq!(outcome.tree.unwrap().size, 7);
    assert_eq!(
        std::fs::read(outside.path().join("outside.bin"))
            .unwrap()
            .len(),
        100
    );
}
