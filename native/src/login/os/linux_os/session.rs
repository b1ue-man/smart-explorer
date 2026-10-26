//! Actual logind seat/class/type plus a usable, pinned local X11 credential.
use std::{fs, io::{self, Read}, os::unix::fs::MetadataExt, path::PathBuf, sync::atomic::{AtomicBool, Ordering}};
use zbus::{blocking::{Connection, Proxy}, zvariant::OwnedObjectPath};
use sha2::{Digest, Sha256};
use crate::login::{core::model::{DesktopKind, SessionBinding}, os::shared::{denied, installation::Installation}};
use super::protected::open_owned;
static END: AtomicBool = AtomicBool::new(false);
extern "C" fn terminate(_: libc::c_int) { END.store(true, Ordering::SeqCst); }
pub(super) struct Greeter { pub binding: SessionBinding, pub uid: u32, pub gid: u32, pub display: String, pub authority: Vec<u8> }
fn property<T>(proxy: &Proxy<'_>, key: &str) -> io::Result<T>
where T: serde::de::DeserializeOwned + TryFrom<zbus::zvariant::OwnedValue>, T::Error: Into<zbus::Error> {
    proxy.get_property(key).map_err(io::Error::other)
}
fn local_display(value: &str) -> bool {
    value.strip_prefix(':').is_some_and(|rest| !rest.is_empty() && rest.len() <= 20 && rest.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
}
pub(super) fn inspect() -> io::Result<Greeter> {
    let connection = Connection::system().map_err(io::Error::other)?;
    let seat = Proxy::new(&connection, "org.freedesktop.login1", "/org/freedesktop/login1/seat/seat0", "org.freedesktop.login1.Seat").map_err(io::Error::other)?;
    let (id, path): (String, OwnedObjectPath) = property(&seat, "ActiveSession")?;
    if id.is_empty() { return Err(denied("seat0 has no active graphical session")); }
    let session = Proxy::new(&connection, "org.freedesktop.login1", path.as_str(), "org.freedesktop.login1.Session").map_err(io::Error::other)?;
    let class: String = property(&session, "Class")?; let kind: String = property(&session, "Type")?;
    let active: bool = property(&session, "Active")?; let remote: bool = property(&session, "Remote")?;
    let (uid, _): (u32, OwnedObjectPath) = property(&session, "User")?;
    let leader: u32 = property(&session, "Leader")?;
    let timestamp: u64 = property(&session, "TimestampMonotonic")?;
    let mut binding = SessionBinding { session_id: id.clone(), user_id: if class == "greeter" { None } else { Some(uid.to_string()) },
        session_generation: format!("{id}:{timestamp}:{uid}:{leader}"), desktop_generation: format!("{id}:{timestamp}:{kind}"),
        desktop: if !active || remote { DesktopKind::Unavailable } else if class != "greeter" { DesktopKind::UserSession }
        else if kind == "wayland" { DesktopKind::WaylandGreeter } else if kind == "x11" { DesktopKind::X11Greeter } else { DesktopKind::Unavailable } };
    if binding.desktop == DesktopKind::X11Greeter && uid == 0 { return Err(denied("root-owned X11 greeter is unsupported; an unprivileged greeter account is required")); }
    if binding.desktop != DesktopKind::X11Greeter { return Ok(Greeter { binding, uid, gid: 0, display: String::new(), authority: Vec::new() }); }
    let display: String = property(&session, "Display")?;
    if !local_display(&display) || leader == 0 { return Err(denied("X11 greeter has no verified local display or process")); }
    let status = fs::read_to_string(format!("/proc/{leader}/status"))?;
    let line = |name: &str| -> io::Result<u32> { status.lines().find_map(|line| line.strip_prefix(name)).and_then(|line| line.split_whitespace().next()).ok_or_else(|| denied("greeter process identity is missing"))?.parse().map_err(io::Error::other) };
    let process_uid = line("Uid:")?; let gid = line("Gid:")?;
    if process_uid != uid && process_uid != 0 { return Err(denied("logind greeter process has an unexpected UID")); }
    let mut environ = Vec::new(); fs::File::open(format!("/proc/{leader}/environ"))?.take(1024 * 1024 + 1).read_to_end(&mut environ)?;
    if environ.len() > 1024 * 1024 { return Err(denied("greeter process environment exceeds limit")); }
    let value = |name: &[u8]| environ.split(|byte| *byte == 0).find_map(|entry| entry.strip_prefix(name)).map(|bytes| String::from_utf8_lossy(bytes).into_owned());
    if value(b"WAYLAND_DISPLAY=").is_some_and(|value| !value.is_empty()) { return Err(denied("greeter is Wayland; XWayland does not authorize login input")); }
    if value(b"DISPLAY=").as_deref() != Some(display.as_str()) { return Err(denied("logind display and actual greeter process display differ")); }
    let authority = PathBuf::from(value(b"XAUTHORITY=").ok_or_else(|| denied("X11 greeter leader exposes no XAUTHORITY binding"))?);
    let pinned = open_owned(&authority, uid, false)?;
    if pinned.file.metadata()?.len() > 64 * 1024 { return Err(denied("greeter Xauthority exceeds limit")); }
    let mut bytes = Vec::new(); pinned.file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > 64 * 1024 { return Err(denied("greeter Xauthority is empty or exceeds limit")); }
    let hash: String = Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    binding.desktop_generation = format!("{}:{display}:{hash}", binding.desktop_generation);
    // Gid from the account database, not a privileged gdm-session-worker's real Gid.
    let gid = if process_uid == uid { gid } else { account_gid(uid)? };
    Ok(Greeter { binding, uid, gid, display, authority: bytes })
}
fn account_gid(uid: u32) -> io::Result<u32> {
    let mut record: libc::passwd = unsafe { std::mem::zeroed() }; let mut result = std::ptr::null_mut(); let mut bytes = vec![0u8; 64 * 1024];
    let code = unsafe { libc::getpwuid_r(uid, &mut record, bytes.as_mut_ptr().cast(), bytes.len(), &mut result) };
    if code != 0 || result.is_null() { return Err(denied("greeter account gid is unavailable")); } Ok(record.pw_gid)
}
pub(crate) fn binding(_: &Installation) -> io::Result<SessionBinding> { inspect().map(|facts| facts.binding) }
fn public_binding() -> io::Result<(SessionBinding, u32, String)> {
    let connection = Connection::system().map_err(io::Error::other)?;
    let seat = Proxy::new(&connection, "org.freedesktop.login1", "/org/freedesktop/login1/seat/seat0", "org.freedesktop.login1.Seat").map_err(io::Error::other)?;
    let (id, path): (String, OwnedObjectPath) = property(&seat, "ActiveSession")?;
    if id.is_empty() { return Err(denied("seat0 has no active greeter")); }
    let session = Proxy::new(&connection, "org.freedesktop.login1", path.as_str(), "org.freedesktop.login1.Session").map_err(io::Error::other)?;
    let class: String = property(&session, "Class")?; let kind: String = property(&session, "Type")?;
    let active: bool = property(&session, "Active")?; let remote: bool = property(&session, "Remote")?;
    let (uid, _): (u32, OwnedObjectPath) = property(&session, "User")?;
    let leader: u32 = property(&session, "Leader")?; let timestamp: u64 = property(&session, "TimestampMonotonic")?;
    let display: String = property(&session, "Display")?;
    if class != "greeter" || kind != "x11" || !active || remote || !local_display(&display) { return Err(denied("the active seat is no longer the local X11 greeter")); }
    Ok((SessionBinding { session_id: id.clone(), user_id: None,
        session_generation: format!("{id}:{timestamp}:{uid}:{leader}"),
        desktop_generation: format!("{id}:{timestamp}:{kind}:{display}"), desktop: DesktopKind::X11Greeter }, uid, display))
}
fn inherited_authority_hash() -> io::Result<String> {
    use std::os::fd::AsRawFd;
    let path = std::env::var("XAUTHORITY").map_err(|_| denied("helper has no inherited Xauthority"))?;
    let fd: i32 = path.strip_prefix("/proc/self/fd/").ok_or_else(|| denied("helper Xauthority is not an inherited descriptor"))?.parse().map_err(io::Error::other)?;
    if fd <= 2 { return Err(denied("helper Xauthority descriptor is invalid")); }
    let mut file = fs::File::open(&path)?;
    let seals = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GET_SEALS) };
    let required = libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
    if seals < 0 || seals & required != required || file.metadata()?.uid() != 0 { return Err(denied("helper Xauthority is not a root-created sealed descriptor")); }
    let mut bytes = Vec::new(); file.by_ref().take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > 64 * 1024 { return Err(denied("helper Xauthority exceeds limit")); }
    Ok(Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect())
}
pub(crate) fn prepare_helper(expected: &SessionBinding, parent: u32, _: bool) -> io::Result<SessionBinding> {
    if unsafe { libc::getppid() } != parent as i32 { return Err(denied("login helper parent changed")); }
    let status = fs::read_to_string(format!("/proc/{parent}/status"))?;
    if !status.lines().any(|line| line.strip_prefix("Uid:").is_some_and(|value| value.split_whitespace().take(4).all(|uid| uid == "0"))) { return Err(denied("login helper parent is not the root broker")); }
    let current = fs::read_link("/proc/self/exe")?;
    let _image = super::protected::open_protected(&current, true)?;
    // /proc/root-process/exe may be unreadable after dropping UID; the root
    // parent and sealed descriptor are checked independently of that proc link.
    let (mut actual, uid, display) = public_binding()?;
    if std::env::var("DISPLAY").ok().as_deref() != Some(display.as_str()) { return Err(denied("helper display differs from the active logind greeter")); }
    if unsafe { libc::geteuid() } != uid || uid == 0 { return Err(denied("helper is not the unprivileged X11 greeter identity")); }
    actual.desktop_generation.push(':'); actual.desktop_generation.push_str(&inherited_authority_hash()?);
    if actual != *expected { return Err(denied("inherited Xauthority differs from the authorized logind binding")); }
    unsafe { libc::signal(libc::SIGTERM, terminate as libc::sighandler_t); libc::signal(libc::SIGINT, terminate as libc::sighandler_t); }
    Ok(actual)
}
pub(crate) fn helper_binding_matches(expected: &SessionBinding) -> io::Result<bool> {
    if END.load(Ordering::SeqCst) { return Ok(false); }
    let (actual, _, _) = match public_binding() { Ok(actual) => actual, Err(_) => return Ok(false) };
    Ok(actual.session_id == expected.session_id && actual.session_generation == expected.session_generation
        && expected.desktop_generation.starts_with(&format!("{}:", actual.desktop_generation)))
}
pub(crate) fn finish_helper() {}
