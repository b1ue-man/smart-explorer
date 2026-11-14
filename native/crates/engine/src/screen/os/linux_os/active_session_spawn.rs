//! Starting a process in the followed session (E23, S3): the screen host
//! process and the indicator window of a worker without session variables
//! run with the session's variables and, when the worker is root, as the
//! session's account (its groups, then its group, then its user; a process
//! that still has effective UID 0 afterwards does not start). The
//! environment of the worker is not passed on except for the variables of
//! `KEPT`, so nothing of root's environment reaches the session. A worker
//! with its own session variables starts its processes as before.
use std::ffi::CString;
use std::io;
use std::os::unix::process::CommandExt;
use std::process::Command;

use super::ActiveSession;

/// Variables of the worker a process in the session keeps.
const KEPT: [&str; 5] = ["PATH", "LANGUAGE", "TZ", "RUST_LOG", "RUST_BACKTRACE"];
/// Largest number of supplementary groups taken over.
const MAX_GROUPS: usize = 1024;

/// The account of a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub uid: u32,
    pub gid: u32,
    pub name: String,
    pub home: String,
    pub shell: String,
    /// Supplementary groups (`getgrouplist`).
    pub groups: Vec<u32>,
}

/// The account database.
pub trait Accounts {
    fn account(&self, uid: u32) -> Option<Account>;
}

/// The system account database (NSS).
pub struct SystemAccounts;

impl Accounts for SystemAccounts {
    fn account(&self, uid: u32) -> Option<Account> {
        let mut record: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result = std::ptr::null_mut();
        let mut buffer = vec![0u8; 64 * 1024];
        // SAFETY: record, buffer and result live for the call; the buffer
        // length is its real length.
        let code = unsafe {
            libc::getpwuid_r(
                uid,
                &mut record,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if code != 0 || result.is_null() || record.pw_name.is_null() {
            return None;
        }
        let text = |pointer: *const libc::c_char| -> String {
            if pointer.is_null() {
                return String::new();
            }
            // SAFETY: getpwuid_r filled the strings NUL-terminated in buffer.
            unsafe { std::ffi::CStr::from_ptr(pointer) }
                .to_string_lossy()
                .into_owned()
        };
        let name = text(record.pw_name);
        let groups = groups_of(&name, record.pw_gid)?;
        Some(Account {
            uid,
            gid: record.pw_gid,
            home: text(record.pw_dir),
            shell: text(record.pw_shell),
            name,
            groups,
        })
    }
}

fn groups_of(name: &str, gid: u32) -> Option<Vec<u32>> {
    let name = CString::new(name).ok()?;
    let mut groups = vec![0 as libc::gid_t; MAX_GROUPS];
    let mut count = MAX_GROUPS as libc::c_int;
    // SAFETY: groups holds `count` entries; getgrouplist writes at most
    // that many and stores the real number in count.
    let found = unsafe { libc::getgrouplist(name.as_ptr(), gid, groups.as_mut_ptr(), &mut count) };
    if found < 0 {
        return None;
    }
    groups.truncate(usize::try_from(count).ok()?);
    Some(groups)
}

/// Whether a process with this environment has session variables of its
/// own; then it starts its processes unchanged.
pub fn has_own_session(env: &dyn Fn(&str) -> Option<String>) -> bool {
    super::super::detect(env) != super::super::SessionKind::None
}

/// The session a process of this worker starts in: `None` for a worker
/// with own session variables (unchanged path) and when no session is
/// followed (then the process finds no display, `no_display`).
pub fn for_worker() -> Option<ActiveSession> {
    if has_own_session(&|name| std::env::var(name).ok()) {
        return None;
    }
    super::current().ok()
}

/// Whether a process started for `generation` still runs in the active
/// session of `seat0`.
pub fn still_current(generation: &str) -> bool {
    super::is_current(generation, super::current_generation().as_deref())
}

/// The environment a process started in `session` gets, given the
/// worker's own variables in `inherited`.
pub fn environment(
    session: &ActiveSession,
    inherited: impl IntoIterator<Item = (String, String)>,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = inherited
        .into_iter()
        .filter(|(name, _)| KEPT.contains(&name.as_str()) || name.starts_with("LC_"))
        .collect();
    let account = &session.account;
    env.push(("HOME".into(), account.home.clone()));
    env.push(("USER".into(), account.name.clone()));
    env.push(("LOGNAME".into(), account.name.clone()));
    if !account.shell.is_empty() {
        env.push(("SHELL".into(), account.shell.clone()));
    }
    env.extend(session.env.iter().cloned());
    env
}

/// Prepares `command` to start in `session`; call it before the caller
/// adds its own variables.
pub fn apply(session: &ActiveSession, command: &mut Command) {
    command.env_clear();
    command.envs(environment(session, std::env::vars()));
    command.current_dir("/");
    // SAFETY: geteuid takes no arguments and cannot fail.
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    let uid = session.account.uid;
    let gid = session.account.gid;
    let groups: Vec<libc::gid_t> = session.account.groups.clone();
    // SAFETY: the closure runs between fork and exec and calls only the
    // async-signal-safe setgroups, setgid, setuid and geteuid on memory
    // that was allocated before the fork.
    unsafe {
        command.pre_exec(move || {
            if libc::setgroups(groups.len(), groups.as_ptr()) != 0
                || libc::setgid(gid) != 0
                || libc::setuid(uid) != 0
            {
                return Err(io::Error::last_os_error());
            }
            if libc::geteuid() == 0 || libc::getegid() != gid {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            Ok(())
        });
    }
}
