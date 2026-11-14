//! The account of the followed session as a worker reads its files (E47,
//! S3): its home from the account database (`spawn::Account`), its UID from
//! logind. A file below the home is read only when every directory from the
//! home down and the file itself belong to that UID and none of them is a
//! link: each step is opened with `O_NOFOLLOW` relative to the one before
//! (`openat`), so a link put in place after a check is not followed either,
//! and the owner is checked on the opened descriptor. A file that is not a
//! regular file (a FIFO, a device) is not read; `O_NONBLOCK` keeps the open
//! itself from waiting. A directory of the account that fails these checks
//! is dropped from a search path; directories outside the home (the system
//! ones) stay as they are. Nothing here writes.
use std::ffi::CString;
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use crate::screen::os::portal_tokens::{self, Tokens, FILE_NAME};
use crate::screen::os::session_detect::active_session::ActiveSession;

/// The program's data directory below the home: `support_dirs::app_data_dir`
/// of an account without `XDG_DATA_HOME` (assumption: the account sets no
/// other data home; the host process of the session starts without one).
pub const APP_DATA: &str = ".local/share/smart_explorer";
/// Largest token file read.
const MAX_TOKEN_FILE: u64 = 64 * 1024;

#[cfg(test)]
#[path = "bildschirm_umgebung_task_tests.rs"]
mod umgebung_tests;

/// The account of a followed session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FollowedAccount {
    uid: u32,
    home: PathBuf,
}

/// Opens `name` relative to `parent` (or the absolute `name` without one)
/// without following a link in its last step.
fn open_at(parent: Option<&OwnedFd>, name: &Path, flags: libc::c_int) -> Option<File> {
    let name = CString::new(name.as_os_str().as_bytes()).ok()?;
    let flags = flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
    let at = parent.map_or(libc::AT_FDCWD, AsRawFd::as_raw_fd);
    // SAFETY: `name` is NUL-terminated and lives for the call; `at` is an
    // open descriptor or AT_FDCWD; openat returns a new descriptor or -1.
    let fd = unsafe { libc::openat(at, name.as_ptr(), flags) };
    if fd < 0 {
        return None;
    }
    // SAFETY: fd is a new descriptor owned by nobody else.
    Some(File::from(unsafe { OwnedFd::from_raw_fd(fd) }))
}

impl FollowedAccount {
    /// The account `uid` with home `home`; `None` for a home that is not
    /// absolute or names `.` or `..`.
    pub fn new(uid: u32, home: impl Into<PathBuf>) -> Option<Self> {
        let home = home.into();
        let plain = home
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)));
        (home.is_absolute() && plain).then_some(Self { uid, home })
    }

    /// The account of `session`.
    pub fn of(session: &ActiveSession) -> Option<Self> {
        Self::new(session.account.uid, &session.account.home)
    }

    /// Whether an opened step is a directory (or a regular file for the
    /// last one) of the account.
    fn owned(&self, file: &File, directory: bool) -> bool {
        file.metadata().is_ok_and(|metadata| {
            let kind = metadata.file_type();
            metadata.uid() == self.uid
                && if directory {
                    kind.is_dir()
                } else {
                    kind.is_file()
                }
        })
    }

    /// The directory `relative` below the home, opened step by step; every
    /// step a directory of the account.
    fn open_dir(&self, relative: &Path) -> Option<OwnedFd> {
        let home = open_at(None, &self.home, libc::O_RDONLY | libc::O_DIRECTORY)?;
        if !self.owned(&home, true) {
            return None;
        }
        let mut at = OwnedFd::from(home);
        for part in relative.components() {
            let Component::Normal(name) = part else {
                return None;
            };
            let next = open_at(
                Some(&at),
                Path::new(name),
                libc::O_RDONLY | libc::O_DIRECTORY,
            )?;
            if !self.owned(&next, true) {
                return None;
            }
            at = OwnedFd::from(next);
        }
        Some(at)
    }

    /// Whether `relative` below the home is a directory of the account with
    /// no link on the way down.
    pub fn owns_dir(&self, relative: &Path) -> bool {
        self.open_dir(relative).is_some()
    }

    /// The regular file `relative` below the home, at most `max` bytes, when
    /// it and every directory on the way are the account's and no link.
    pub fn read_owned(&self, relative: &Path, max: u64) -> Option<Vec<u8>> {
        let name = relative.file_name()?;
        let parent = self.open_dir(relative.parent()?)?;
        let file = open_at(Some(&parent), Path::new(name), libc::O_RDONLY)?;
        if !self.owned(&file, false) || file.metadata().ok()?.len() > max {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(max + 1).read_to_end(&mut bytes).ok()?;
        (bytes.len() as u64 <= max).then_some(bytes)
    }

    /// The portal tokens of the account; none when the file is missing,
    /// not the account's, reached over a link or not valid.
    pub fn tokens(&self) -> Tokens {
        self.read_owned(&Path::new(APP_DATA).join(FILE_NAME), MAX_TOKEN_FILE)
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|text| portal_tokens::parse(&text))
            .unwrap_or_default()
    }

    /// `dirs` without the directories below the home that are not the
    /// account's or are reached over a link; the others unchanged.
    pub fn trusted_dirs(&self, dirs: Vec<PathBuf>) -> Vec<PathBuf> {
        dirs.into_iter()
            .filter(|dir| match dir.strip_prefix(&self.home) {
                Ok(relative) => self.owns_dir(relative),
                Err(_) => true,
            })
            .collect()
    }
}

/// The portal tokens of the followed session's account; none without one.
pub fn followed_tokens() -> Tokens {
    crate::screen::os::session_detect::active_session::spawn::for_worker()
        .and_then(|session| FollowedAccount::of(&session))
        .map(|account| account.tokens())
        .unwrap_or_default()
}

/// The account a host process of a worker that follows a session runs as
/// (its mark `SE_SCREEN_FOLLOWED_SESSION`, its `HOME`, its effective UID);
/// `None` in any other process.
pub fn started_host_account() -> Option<FollowedAccount> {
    use crate::screen::os::session_detect::active_session::process;
    std::env::var_os(process::HOST_ENV)?;
    let home = std::env::var_os("HOME")?;
    // SAFETY: geteuid takes no arguments and cannot fail.
    FollowedAccount::new(unsafe { libc::geteuid() }, PathBuf::from(home))
}
