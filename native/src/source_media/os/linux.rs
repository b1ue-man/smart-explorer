//! Resolve the mounted device through st_dev/sysfs, including bind aliases.
use std::{fs, io, os::unix::fs::MetadataExt, path::Path};
use super::Source;

pub(super) fn probe(path: &Path) -> io::Result<Option<Source>> {
    // Preserve the existing consented local metadata boundary; a plain stat
    // must not reject a path that local_access can inspect after its grant.
    let metadata = crate::local_access::symlink_metadata(path)?;
    let metadata = if metadata.file_type().is_symlink() { fs::metadata(path)? } else { metadata };
    probe_device(metadata.dev())
}

pub(super) fn probe_file(file: &fs::File) -> io::Result<Option<Source>> {
    #[cfg(test)]
    {
        use std::os::fd::AsRawFd;
        if let Ok(path) = fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())) {
            if let Some(source) = super::test_profiles::probe(&path) { return Ok(Some(source)); }
        }
    }
    probe_device(file.metadata()?.dev())
}

fn probe_device(device: u64) -> io::Result<Option<Source>> {
    let key = format!("{}:{}", libc::major(device), libc::minor(device));
    let sys = match fs::canonicalize(format!("/sys/dev/block/{key}")) {
        Ok(sys) => sys,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    // TYPE_ROM (5) is SCSI device evidence; a loop-mounted udf image has no
    // such device/type. A partition resolves to its enclosing optical disk,
    // so separate mounted partitions cannot bypass the physical reader lock.
    let Some(disk) = sys.ancestors().take(3).find(|candidate|
        fs::read_to_string(candidate.join("device/type")).is_ok_and(|kind| kind.trim() == "5")) else { return Ok(None); };
    let sequence = fs::read_to_string(disk.join("diskseq"))?;
    if sequence.trim().parse::<u64>().is_err() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "optical device has no valid media sequence"));
    }
    let mounts = fs::read_to_string("/proc/self/mountinfo")?;
    let mut ids = Vec::new();
    let mut read_only = true;
    for line in mounts.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() > 5 && fields[2] == key {
            ids.push(fields[0]);
            read_only &= fields[5].split(',').any(|option| option == "ro");
        }
    }
    if ids.is_empty() { return Err(io::Error::other("optical medium is no longer mounted")); }
    Ok(Some(Source {
        device: format!("linux-optical:{}", disk.display()),
        generation: format!("boot:{}:diskseq:{}:volume:{key}", boot_id()?, sequence.trim()),
        read_only,
    }))
}

pub(super) fn try_lock(file: &fs::File, exclusive: bool) -> io::Result<bool> {
    use std::os::fd::AsRawFd;
    let operation = if exclusive { libc::LOCK_EX } else { libc::LOCK_SH };
    if unsafe { libc::flock(file.as_raw_fd(), operation | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::WouldBlock { Ok(false) } else { Err(error) }
}

pub(super) fn unlock(file: &fs::File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) } == 0 { Ok(()) }
    else { Err(io::Error::last_os_error()) }
}

fn boot_id() -> io::Result<&'static str> {
    static BOOT: std::sync::OnceLock<Result<String, String>> = std::sync::OnceLock::new();
    match BOOT.get_or_init(|| fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|id| id.trim().to_string()).map_err(|error| error.to_string())) {
        Ok(id) if id.len() == 36 && id.bytes().all(|byte| byte.is_ascii_hexdigit() || byte == b'-') => Ok(id),
        Ok(_) => Err(io::Error::other("OS boot identifier is not valid")),
        Err(error) => Err(io::Error::other(error.clone())),
    }
}
