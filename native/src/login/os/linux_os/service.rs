//! AF_UNIX only. Kernel peer credentials must match the exact dedicated UID.
use std::{ffi::CString, fs::{self, File, OpenOptions}, io, os::{fd::AsRawFd, unix::{fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt}, net::{UnixListener, UnixStream}}}, path::Path, sync::atomic::{AtomicBool, Ordering}, time::Duration};
use crate::login::os::shared::{broker, denied, installation, nonce};
pub(crate) type ClientStream = UnixStream;
const SOCKET: &str = "/run/smart-explorer-login/broker.sock";
static STOP: AtomicBool = AtomicBool::new(false);
extern "C" fn stop(_: libc::c_int) { STOP.store(true, Ordering::SeqCst); }
pub(super) fn gateway_account() -> io::Result<(u32, u32)> {
    let name = CString::new("smart-explorer-login").map_err(io::Error::other)?;
    let mut record: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut(); let mut bytes = vec![0u8; 64 * 1024];
    let code = unsafe { libc::getpwnam_r(name.as_ptr(), &mut record, bytes.as_mut_ptr().cast(), bytes.len(), &mut result) };
    if code != 0 { return Err(io::Error::from_raw_os_error(code)); }
    if result.is_null() || record.pw_uid == 0 { return Err(denied("dedicated smart-explorer-login account is not installed")); }
    Ok((record.pw_uid, record.pw_gid))
}
fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut credentials as *mut libc::ucred).cast(), &mut len) };
    if result != 0 || len as usize != std::mem::size_of::<libc::ucred>() { return Err(io::Error::last_os_error()); }
    Ok(credentials.uid)
}
pub(crate) fn connect() -> io::Result<UnixStream> {
    if unsafe { libc::geteuid() } != gateway_account()?.0 { return Err(denied("login IPC client must be the dedicated gateway UID")); }
    let stream = UnixStream::connect(SOCKET)?;
    if peer_uid(&stream)? != 0 { return Err(denied("login broker socket is not served by root")); }
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?; Ok(stream)
}
pub(crate) fn run_broker() -> io::Result<()> {
    if unsafe { libc::geteuid() } != 0 { return Err(denied("login broker requires root")); }
    let initial = installation::load()?;
    let current = File::open("/proc/self/exe")?;
    if !same_file::is_same_file("/proc/self/exe", &initial.image_path)? { return Err(denied("login broker is not the protected release image")); }
    installation::verify_hash(&current, &initial.manifest.sha256)?;
    let (uid, gid) = gateway_account()?;
    let parent = Path::new(SOCKET).parent().ok_or_else(|| denied("missing login runtime parent"))?;
    let directory = fs::symlink_metadata(parent)?;
    if !directory.is_dir() || directory.uid() != 0 || directory.mode() & 0o022 != 0 { return Err(denied("login runtime directory must be root-owned without group write")); }
    let lock = OpenOptions::new().write(true).create(true).mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(parent.join("broker.lock"))?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 { return Err(io::Error::other("a login broker already owns the runtime lock")); }
    if let Ok(meta) = fs::symlink_metadata(SOCKET) {
        if !meta.file_type().is_socket() || meta.uid() != 0 { return Err(denied("untrusted existing login socket")); }
        fs::remove_file(SOCKET)?;
    }
    let listener = UnixListener::bind(SOCKET)?;
    let socket = CString::new(SOCKET).map_err(io::Error::other)?;
    if unsafe { libc::chown(socket.as_ptr(), 0, gid) } != 0 { return Err(io::Error::last_os_error()); }
    fs::set_permissions(SOCKET, fs::Permissions::from_mode(0o660))?;
    listener.set_nonblocking(true)?;
    unsafe { libc::signal(libc::SIGTERM, stop as libc::sighandler_t); libc::signal(libc::SIGINT, stop as libc::sighandler_t); }
    let boot = format!("{}:{}", fs::read_to_string("/proc/sys/kernel/random/boot_id")?.trim(), nonce()?);
    super::super::shared::host_status::publish("idle", &boot, &initial.policy)?;
    super::readiness::notify()?;
    let mut status_at = std::time::Instant::now() - Duration::from_secs(2);
    while !STOP.load(Ordering::SeqCst) {
        if status_at.elapsed() >= Duration::from_secs(1) { super::super::shared::host_status::publish("idle", &boot, &installation::policy()?)?; status_at = std::time::Instant::now(); }
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                stream.set_write_timeout(Some(Duration::from_secs(5)))?;
                if peer_uid(&stream)? == uid { if let Err(error) = broker::serve(&mut stream, &boot) { let _ = crate::login::core::wire::send(&mut stream, &crate::login::core::wire::Reply::refused(error)); } }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => return Err(error),
        }
    }
    super::super::shared::host_status::publish("stopped", &boot, &installation::policy()?)?;
    drop(listener); fs::remove_file(SOCKET)?; Ok(())
}
