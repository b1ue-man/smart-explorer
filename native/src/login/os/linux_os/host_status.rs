//! Read-only public root evidence; socket/privileged operation permissions stay separate.
use super::protected;
use std::{
    fs::{File, OpenOptions, Permissions},
    io::{self, Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
};
const ROOT: &str = "/var/lib/smart-explorer-login-status";
pub(crate) fn host_process_instance(pid: u32) -> io::Result<String> {
    let path = format!("/proc/{pid}");
    let mut stat = String::new();
    File::open(format!("{path}/stat"))?
        .take(4096)
        .read_to_string(&mut stat)?;
    let tail = stat
        .rsplit_once(')')
        .map(|(_, tail)| tail)
        .ok_or_else(|| io::Error::other("process state is unavailable"))?;
    if tail
        .split_whitespace()
        .next()
        .is_some_and(|state| matches!(state, "Z" | "X" | "x"))
    {
        return Err(io::Error::other(
            "login activity process is no longer alive",
        ));
    }
    let start = tail
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| io::Error::other("process start generation is unavailable"))?;
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    Ok(format!("{}:{}", boot.trim(), start))
}
pub(crate) fn host_uptime_ms() -> io::Result<u64> {
    let mut value: libc::timespec = unsafe { std::mem::zeroed() };
    if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut value) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((value.tv_sec as u64)
        .saturating_mul(1000)
        .saturating_add(value.tv_nsec as u64 / 1_000_000))
}
pub(crate) fn read_host_evidence() -> io::Result<Vec<u8>> {
    let pin = protected::open_protected(&Path::new(ROOT).join("status.json"), false)?;
    if pin.file.metadata()?.len() > 4096 {
        return Err(io::Error::other("public broker status exceeds limit"));
    }
    let mut bytes = Vec::new();
    pin.file.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(io::Error::other("public broker status exceeds limit"));
    }
    Ok(bytes)
}
pub(crate) fn write_host_evidence(bytes: &[u8]) -> io::Result<()> {
    if unsafe { libc::geteuid() } != 0 || bytes.len() > 4096 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "public broker evidence requires root and bounded data",
        ));
    }
    let root = Path::new(ROOT);
    let _pin = protected::open_protected(&root.join("owner.marker"), false)?;
    let temp = root.join(format!(".status-{}", crate::login::os::shared::nonce()?));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&temp)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.set_permissions(Permissions::from_mode(0o644))?;
        file.sync_all()?;
        std::fs::rename(&temp, root.join("status.json"))?;
        File::open(root)?.sync_all()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
