//! Resolves a grant's run-as account on a Linux host (`getpwnam_r(3)`,
//! `getgrouplist(3)`), in the root worker before the start is released. The
//! supervisor then switches the job's root process to exactly these ids
//! (`exec_supervisor.rs`), so it never consults the user database itself
//! between `fork` and `exec`.
use std::ffi::{CStr, CString};
use std::io;

use crate::share::exec_supervisor_protocol::RunAsAccount;

/// Start buffer of `getpwnam_r`; doubled on `ERANGE` up to `MAX_BUFFER`.
const START_BUFFER: usize = 16 * 1024;
const MAX_BUFFER: usize = 1024 * 1024;
/// glibc's own bound for supplementary groups (`NGROUPS_MAX` is 65536).
const MAX_GROUPS: usize = 65_536;
const DEFAULT_SHELL: &str = "/bin/sh";

pub(super) fn resolve(name: &str) -> io::Result<RunAsAccount> {
    let c_name = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "account name contains NUL"))?;
    let mut buffer = vec![0u8; START_BUFFER];
    loop {
        // SAFETY: `passwd` is plain data; `getpwnam_r` fills it with pointers
        // into `buffer`, which outlives every use below.
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let status = unsafe {
            libc::getpwnam_r(
                c_name.as_ptr(),
                &mut entry,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if status == libc::ERANGE && buffer.len() < MAX_BUFFER {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        if result.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("local account {name:?} does not exist on this host"),
            ));
        }
        let home = text(entry.pw_dir).unwrap_or_else(|| "/".to_string());
        let shell = text(entry.pw_shell)
            .filter(|shell| shell.starts_with('/'))
            .unwrap_or_else(|| DEFAULT_SHELL.to_string());
        let groups = group_list(&c_name, entry.pw_gid)?;
        return Ok(RunAsAccount {
            name: name.to_string(),
            uid: entry.pw_uid,
            gid: entry.pw_gid,
            groups,
            home: if home.is_empty() { "/".into() } else { home },
            shell,
        });
    }
}

fn text(pointer: *const libc::c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: a non-null field of a filled `passwd` is a NUL-terminated string.
    let value = unsafe { CStr::from_ptr(pointer) };
    Some(value.to_string_lossy().into_owned())
}

/// The primary group and every supplementary group of the account.
fn group_list(name: &CStr, gid: libc::gid_t) -> io::Result<Vec<u32>> {
    let mut capacity: usize = 64;
    loop {
        let mut groups = vec![0 as libc::gid_t; capacity];
        let mut count = capacity as libc::c_int;
        // SAFETY: `groups` holds `count` entries; glibc writes at most that
        // many and reports the needed number in `count` when it is too small.
        let status =
            unsafe { libc::getgrouplist(name.as_ptr(), gid, groups.as_mut_ptr(), &mut count) };
        let needed = usize::try_from(count).unwrap_or(0);
        if status >= 0 {
            groups.truncate(needed.min(capacity));
            if !groups.contains(&gid) {
                groups.insert(0, gid);
            }
            return Ok(groups);
        }
        if needed <= capacity || needed > MAX_GROUPS {
            return Err(io::Error::other(
                "the group list of the run-as account could not be read",
            ));
        }
        capacity = needed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luecken_task_root_and_missing_accounts_resolve_as_the_user_database_says() {
        let root = resolve("root").unwrap();
        assert_eq!(root.uid, 0);
        assert_eq!(root.gid, 0);
        assert!(root.groups.contains(&0));
        assert!(root.home.starts_with('/'));
        assert!(root.shell.starts_with('/'));
        let missing = resolve("se-no-such-account-4711").unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    }
}
