//! Linux privilege boundary: root broker, dedicated gateway UID, greeter UID helper.
mod protected;
mod service;
mod readiness;
mod host_status;
pub(crate) use host_status::{host_process_instance, host_uptime_ms, read_host_evidence, write_host_evidence};
mod session;
mod spawn;
pub(crate) use protected::{open_protected, ProtectedFile};
pub(crate) use service::{connect, run_broker, ClientStream};
pub(crate) use session::{binding, finish_helper, helper_binding_matches, prepare_helper};
pub(crate) use spawn::spawn;
use std::{io, path::PathBuf};
use crate::login::{core::model::SessionBinding, os::shared::{denied, installation::Paths}};
pub(crate) fn paths() -> io::Result<Paths> { Ok(Paths { config: PathBuf::from("/var/lib/smart-explorer-login"), releases: PathBuf::from("/usr/libexec/smart-explorer-login/releases"), image_name: "smart-explorer-login" }) }
pub(crate) fn validate_gateway_identity(identity: &str) -> io::Result<()> {
    let uid: u32 = identity.parse().map_err(|_| denied("login gateway identity must be a dedicated numeric UID"))?;
    if uid == 0 || uid != service::gateway_account()?.0 { return Err(denied("login gateway UID must match the dedicated smart-explorer-login system account")); } Ok(())
}
pub(crate) fn secure_attention(_: &SessionBinding) -> io::Result<()> { Err(io::Error::new(io::ErrorKind::Unsupported, "Windows secure attention is unavailable on Linux")) }

pub(crate) fn helper_channel() -> io::Result<(Box<dyn std::io::Read + Send>, Box<dyn std::io::Write + Send>)> { Ok((Box::new(std::io::stdin()), Box::new(std::io::stdout()))) }

pub(crate) fn run_probe(_: Box<dyn std::io::Read + Send>, _: Box<dyn std::io::Write + Send>, _: crate::login::core::wire::HelperInit, _: crate::login::core::model::SessionBinding) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "session probe process is only used by Windows"))
}

pub(crate) fn gateway_state_directory() -> io::Result<PathBuf> { Ok(PathBuf::from("/var/lib/smart-explorer-login-gateway")) }

// Private gateway state and explicit offline provisioning; no profile lookup.
pub(crate) fn require_gateway(policy: &crate::login::Policy) -> io::Result<()> {
    validate_gateway_identity(&policy.gateway_identity)?;
    let uid = unsafe { libc::geteuid() };
    if uid == 0 || uid != service::gateway_account()?.0 { return Err(denied("boot gateway requires the dedicated unprivileged service UID")); }
    Ok(())
}
fn gateway_file(name: &str, max: usize) -> io::Result<Vec<u8>> {
    use std::{io::Read, os::unix::fs::MetadataExt};
    let uid = service::gateway_account()?.0;
    if unsafe { libc::geteuid() } != uid { return Err(denied("private gateway state requires its dedicated UID")); }
    let root = gateway_state_directory()?;
    let meta = std::fs::symlink_metadata(&root)?;
    if !meta.is_dir() || meta.mode() & 0o077 != 0 || (meta.uid() != 0 && meta.uid() != uid) { return Err(denied("gateway state directory is not private")); }
    let pinned = protected::open_owned(&root.join(name), uid, false)?;
    if pinned.file.metadata()?.mode() & 0o077 != 0 || pinned.file.metadata()?.len() > max as u64 { return Err(denied("gateway state file is not private or exceeds its limit")); }
    let mut bytes = Vec::new(); pinned.file.take(max as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max { return Err(denied("private gateway record exceeds limit")); } Ok(bytes)
}
pub(crate) fn read_gateway_key() -> io::Result<Vec<u8>> { gateway_file(crate::login::core::gateway_types::MACHINE_FILE, 8192) }
pub(crate) fn read_gateway_network() -> io::Result<Option<Vec<u8>>> {
    match gateway_file(crate::login::core::gateway_types::NETWORK_FILE, crate::login::core::gateway_types::MAX_NETWORK) { Ok(bytes) => Ok(Some(bytes)), Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None), Err(error) => Err(error) }
}
pub(crate) fn write_gateway_address(bytes: &[u8]) -> io::Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    if bytes.len() > 32768 { return Err(denied("public machine address exceeds limit")); }
    let uid = service::gateway_account()?.0;
    if unsafe { libc::geteuid() } != uid { return Err(denied("gateway address publisher requires its dedicated UID")); }
    let root = gateway_state_directory()?;
    let _pins = protected::open_owned(&root.join(crate::login::core::gateway_types::MACHINE_FILE), uid, false)?;
    let temporary = root.join(format!(".address-{}", crate::login::os::shared::nonce()?));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(&temporary)?;
    let result = (|| { file.write_all(bytes)?; file.sync_all()?; std::fs::rename(&temporary, root.join(crate::login::core::gateway_types::ADDRESS_FILE))?; std::fs::File::open(&root)?.sync_all() })();
    if result.is_err() { let _ = std::fs::remove_file(temporary); } result
}
static GATEWAY_STOP: std::sync::OnceLock<std::sync::Arc<std::sync::atomic::AtomicBool>> = std::sync::OnceLock::new();
extern "C" fn gateway_stop(_: libc::c_int) {
    if let Some(stop) = GATEWAY_STOP.get() { stop.store(true, std::sync::atomic::Ordering::SeqCst); }
}
pub(crate) fn run_gateway_service() -> io::Result<()> {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    GATEWAY_STOP.set(stop.clone()).map_err(|_| denied("gateway lifecycle was already initialized"))?;
    if unsafe { libc::signal(libc::SIGTERM, gateway_stop as libc::sighandler_t) } == libc::SIG_ERR
        || unsafe { libc::signal(libc::SIGINT, gateway_stop as libc::sighandler_t) } == libc::SIG_ERR { return Err(io::Error::last_os_error()); }
    crate::login::os::shared::gateway::run_until_ready(stop, readiness::notify)
}
/// Fresh root-only output directory; no existing file or active policy is overwritten.
pub(crate) fn write_provision_bundle(directory: &std::path::Path, records: &[(&str, &[u8])]) -> io::Result<()> {
    use std::{ffi::CString, fs::File, io::Write, os::{fd::{AsRawFd, FromRawFd}, unix::{ffi::OsStrExt, fs::MetadataExt}}, path::Component};
    if unsafe { libc::geteuid() } != 0 { return Err(denied("login provisioning requires an administrator/root private output directory")); }
    if !directory.is_absolute() { return Err(denied("provisioning output must be absolute")); }
    let components: Vec<_> = directory.components().filter(|part| !matches!(part, Component::RootDir)).collect();
    if components.is_empty() { return Err(denied("provisioning output must be a new child directory")); }
    let mut parents = vec![File::open("/")?];
    for part in &components[..components.len()-1] {
        let Component::Normal(name) = part else { return Err(denied("provisioning path contains traversal")); };
        let name = CString::new(name.as_bytes()).map_err(io::Error::other)?;
        let raw = unsafe { libc::openat(parents.last().ok_or_else(|| denied("missing provisioning parent"))?.as_raw_fd(), name.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
        if raw < 0 { return Err(io::Error::last_os_error()); }
        let file = unsafe { File::from_raw_fd(raw) }; let meta = file.metadata()?;
        if meta.uid() != 0 || meta.mode() & 0o022 != 0 { return Err(denied("provisioning ancestry is writable by an unprivileged user")); }
        parents.push(file);
    }
    let Some(Component::Normal(leaf)) = components.last() else { return Err(denied("invalid provisioning directory name")); };
    let leaf = CString::new(leaf.as_bytes()).map_err(io::Error::other)?;
    let parent = parents.last().ok_or_else(|| denied("missing provisioning parent"))?;
    if unsafe { libc::mkdirat(parent.as_raw_fd(), leaf.as_ptr(), 0o700) } != 0 { return Err(io::Error::last_os_error()); }
    let raw = unsafe { libc::openat(parent.as_raw_fd(), leaf.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if raw < 0 {
        let error = io::Error::last_os_error();
        unsafe { libc::unlinkat(parent.as_raw_fd(), leaf.as_ptr(), libc::AT_REMOVEDIR); }
        return Err(error);
    }
    let root = unsafe { File::from_raw_fd(raw) }; let mut created = Vec::new();
    let result = (|| {
        for (name, bytes) in records {
            if name.contains(['/', '\\', '\0']) || name.is_empty() || bytes.len() > 1048576 { return Err(denied("invalid private provisioning record")); }
            let name = CString::new(*name).map_err(io::Error::other)?;
            let raw = unsafe { libc::openat(root.as_raw_fd(), name.as_ptr(), libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC, 0o600) };
            if raw < 0 { return Err(io::Error::last_os_error()); }
            let mut file = unsafe { File::from_raw_fd(raw) }; created.push(name);
            file.write_all(bytes)?; file.sync_all()?;
        }
        root.sync_all()?; parent.sync_all()
    })();
    if result.is_err() {
        for name in created { unsafe { libc::unlinkat(root.as_raw_fd(), name.as_ptr(), 0); } }
        unsafe { libc::unlinkat(parent.as_raw_fd(), leaf.as_ptr(), libc::AT_REMOVEDIR); }
    }
    result
}
