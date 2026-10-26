//! logind supplies real UID, session type, active state and login incarnation.
//! Environment DISPLAY/XDG_SESSION_ID alone is never an authorization proof.
use crate::apps::core::model::{HostSession, SessionId, UserId};
use std::{fs, io, path::PathBuf, time::Duration};
use zbus::{
    blocking::{connection::Builder, Proxy},
    zvariant::OwnedObjectPath,
};

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("GUI session: {error}"))
}
pub(super) fn current() -> io::Result<HostSession> {
    let uid = unsafe { libc::geteuid() };
    if uid == 0 || uid != unsafe { libc::getuid() } {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "root/setuid workers cannot start GUI applications",
        ));
    }
    let bus = Builder::system()
        .map_err(failed)?
        .method_timeout(Duration::from_secs(5))
        .build()
        .map_err(failed)?;
    let manager = Proxy::new(
        &bus,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .map_err(failed)?;
    let path: OwnedObjectPath = manager
        .call("GetSessionByPID", &(std::process::id(),))
        .map_err(failed)?;
    let session = Proxy::new(
        &bus,
        "org.freedesktop.login1",
        path.as_str(),
        "org.freedesktop.login1.Session",
    )
    .map_err(failed)?;
    let user: (u32, OwnedObjectPath) = session.get_property("User").map_err(failed)?;
    let active: bool = session.get_property("Active").map_err(failed)?;
    let kind: String = session.get_property("Type").map_err(failed)?;
    let state: String = session.get_property("State").map_err(failed)?;
    let id: String = session.get_property("Id").map_err(failed)?;
    let started: u64 = session.get_property("TimestampMonotonic").map_err(failed)?;
    let locked: bool = session.get_property("LockedHint").map_err(failed)?;
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    if user.0 != uid
        || started == 0
        || !active
        || state != "active"
        || !matches!(kind.as_str(), "x11" | "wayland")
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "process is not in the active graphical login session",
        ));
    }
    Ok(HostSession {
        user_id: UserId::try_from(format!("linux-uid:{uid}"))?,
        session_id: SessionId::try_from(format!("linux-login:{}:{id}:{started}", boot.trim()))?,
        interactive: true,
        unlocked: !locked,
    })
}

pub(super) fn home() -> io::Result<PathBuf> {
    let uid = unsafe { libc::geteuid() };
    let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut output = std::ptr::null_mut();
    let mut bytes = vec![0u8; 64 * 1024];
    let result = unsafe {
        libc::getpwuid_r(
            uid,
            pwd.as_mut_ptr(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            &mut output,
        )
    };
    if result != 0 || output.is_null() {
        return Err(failed("cannot resolve actual interactive user's home"));
    }
    let pwd = unsafe { pwd.assume_init() };
    if pwd.pw_dir.is_null() {
        return Err(failed("interactive user has no home"));
    }
    use std::os::unix::ffi::OsStrExt;
    let home = unsafe { std::ffi::CStr::from_ptr(pwd.pw_dir) };
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(home.to_bytes())))
}
