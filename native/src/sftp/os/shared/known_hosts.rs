use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

const MAX_KNOWN_HOSTS_BYTES: u64 = 1024 * 1024;

fn app_data_dir() -> PathBuf {
    crate::support_dirs::app_data_dir()
}

fn known_hosts_path() -> PathBuf {
    app_data_dir().join("known_hosts_sftp.txt")
}

/// TOFU: accept a matching or first-seen key only after the trust decision is
/// durably stored. Changed keys and any storage/corruption failure fail closed.
pub(super) fn known_hosts_accept(
    host: &str,
    port: u16,
    key: &russh::keys::PublicKey,
) -> io::Result<bool> {
    let fingerprint = key.fingerprint(Default::default()).to_string();
    accept_fingerprint(&known_hosts_path(), &format!("{host}:{port}"), &fingerprint)
}

fn accept_fingerprint(path: &Path, host: &str, fingerprint: &str) -> io::Result<bool> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let lock_path = parent.join("known_hosts_sftp.lock");
    reject_non_regular_if_present(&lock_path, "SFTP host-key lock")?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.lock()?;

    if let Some((_, saved_fingerprint)) = load_known_hosts(path)?
        .into_iter()
        .find(|(saved_host, _)| saved_host == host)
    {
        return Ok(saved_fingerprint == fingerprint);
    }

    reject_non_regular_if_present(path, "SFTP known-hosts file")?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    validate_regular_file(&file, "SFTP known-hosts file")?;
    writeln!(file, "{host} {fingerprint}")?;
    file.flush()?;
    file.sync_all()?;
    Ok(true)
}

fn load_known_hosts(path: &Path) -> io::Result<Vec<(String, String)>> {
    let mut file = match open_existing_regular(path)? {
        Some(file) => file,
        None => return Ok(Vec::new()),
    };
    let metadata = file.metadata()?;
    if metadata.len() > MAX_KNOWN_HOSTS_BYTES {
        return Err(invalid("SFTP known-hosts file exceeds its 1 MiB limit"));
    }
    let capacity = usize::try_from(metadata.len())
        .map_err(|_| invalid("SFTP known-hosts length does not fit this platform"))?;
    let mut bytes = Vec::with_capacity(capacity);
    Read::by_ref(&mut file)
        .take(MAX_KNOWN_HOSTS_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_KNOWN_HOSTS_BYTES {
        return Err(invalid("SFTP known-hosts file exceeds its 1 MiB limit"));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| invalid("SFTP known-hosts file is not valid UTF-8"))?;
    let mut entries: Vec<(String, String)> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 2 {
            return Err(invalid(format!(
                "invalid SFTP known-host entry on line {}",
                index + 1
            )));
        }
        if let Some((_, previous)) = entries.iter().find(|(host, _)| host == fields[0]) {
            if previous != fields[1] {
                return Err(invalid(format!(
                    "conflicting SFTP host keys for {}",
                    fields[0]
                )));
            }
            continue;
        }
        entries.push((fields[0].to_string(), fields[1].to_string()));
    }
    Ok(entries)
}

fn open_existing_regular(path: &Path) -> io::Result<Option<File>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => return Err(permission("SFTP known-hosts file is not a regular file")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    let file = File::open(path)?;
    validate_regular_file(&file, "SFTP known-hosts file")?;
    Ok(Some(file))
}

fn reject_non_regular_if_present(path: &Path, label: &str) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(()),
        Ok(_) => Err(permission(format!("{label} is not a regular file"))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn validate_regular_file(file: &File, label: &str) -> io::Result<()> {
    if file.metadata()?.file_type().is_file() {
        Ok(())
    } else {
        Err(permission(format!("{label} is not a regular file")))
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn permission(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message.into())
}

#[cfg(test)]
mod tests {
    use super::{accept_fingerprint, load_known_hosts, MAX_KNOWN_HOSTS_BYTES};

    #[test]
    fn first_seen_is_durable_and_changed_key_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("known");
        assert!(accept_fingerprint(&path, "host:22", "SHA256:first").unwrap());
        assert!(accept_fingerprint(&path, "host:22", "SHA256:first").unwrap());
        assert!(!accept_fingerprint(&path, "host:22", "SHA256:changed").unwrap());
        assert_eq!(load_known_hosts(&path).unwrap().len(), 1);
    }

    #[test]
    fn malformed_conflicting_and_oversized_files_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("known");
        std::fs::write(&path, "broken\n").unwrap();
        assert!(accept_fingerprint(&path, "host:22", "fingerprint").is_err());
        std::fs::write(&path, "host:22 one\nhost:22 two\n").unwrap();
        assert!(accept_fingerprint(&path, "other:22", "fingerprint").is_err());
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_KNOWN_HOSTS_BYTES + 1)
            .unwrap();
        assert!(accept_fingerprint(&path, "host:22", "fingerprint").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_store_fails_closed_without_touching_target() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let path = directory.path().join("known");
        std::fs::write(&target, "unchanged").unwrap();
        symlink(&target, &path).unwrap();
        assert!(accept_fingerprint(&path, "host:22", "fingerprint").is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "unchanged");
    }
}
