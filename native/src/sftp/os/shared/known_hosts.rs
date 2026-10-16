//! Host-key trust store of the SSH client.
//!
//! Smart Explorer's own pins live in `<App-Daten>/known_hosts_sftp.txt`, one
//! `host:port SHA256:<fingerprint>` line each, written only after a person
//! confirmed the key (or it was found in OpenSSH's files). OpenSSH's
//! `~/.ssh/known_hosts` and the system-wide `ssh_known_hosts` are read too,
//! so a host already trusted with `ssh` is trusted here; they are never
//! written. A changed or revoked key and every storage or corruption error
//! fail closed; an `@revoked` entry of OpenSSH's files refuses a key even
//! when Smart Explorer pinned it. OpenSSH's files are read through links like
//! `ssh` does, only up to 1 MiB each, and without owner or mode checks (as
//! `ssh`); `@cert-authority` lines are skipped.
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use super::host_key::{openssh_host_name, parse_known_hosts_line, patterns_match, HostKeyVerdict};

const MAX_KNOWN_HOSTS_BYTES: u64 = 1024 * 1024;
const STORE_NAME: &str = "known_hosts_sftp.txt";
const LOCK_NAME: &str = "known_hosts_sftp.lock";

fn app_data_dir() -> PathBuf {
    crate::support_dirs::app_data_dir()
}

pub(super) fn fingerprint(key: &russh::keys::PublicKey) -> String {
    key.fingerprint(Default::default()).to_string()
}

/// Decides about `key` presented by `host:port`.
pub(super) fn verify(
    host: &str,
    port: u16,
    key: &russh::keys::PublicKey,
) -> io::Result<HostKeyVerdict> {
    verify_in(&app_data_dir(), &openssh_files(), host, port, key)
}

fn verify_in(
    directory: &Path,
    openssh: &[PathBuf],
    host: &str,
    port: u16,
    key: &russh::keys::PublicKey,
) -> io::Result<HostKeyVerdict> {
    // OpenSSH's files come first: an `@revoked` entry refuses the key even
    // when Smart Explorer pinned it ("never accepted", sshd(8)).
    let recorded = openssh_verdict_all(openssh, &openssh_host_name(host, port), key)?;
    if matches!(recorded, HostKeyVerdict::Revoked { .. }) {
        return Ok(recorded);
    }
    let label = format!("{host}:{port}");
    if let Some(pinned) = pinned_fingerprint(&directory.join(STORE_NAME), &label)? {
        return Ok(if pinned == fingerprint(key) {
            HostKeyVerdict::Trusted
        } else {
            HostKeyVerdict::Changed {
                source: STORE_NAME.into(),
            }
        });
    }
    Ok(recorded)
}

/// What OpenSSH's files say about `key` for `name`, across all of them:
/// revoked beats trusted beats changed beats unknown. OpenSSH accepts a key
/// "if any matching line has the proper key", so a stale key of the same type
/// in the user file does not outvote the system file that lists this one.
fn openssh_verdict_all(
    openssh: &[PathBuf],
    name: &str,
    key: &russh::keys::PublicKey,
) -> io::Result<HostKeyVerdict> {
    let mut verdict = HostKeyVerdict::Unknown;
    for path in openssh {
        match openssh_verdict(path, name, key)? {
            HostKeyVerdict::Unknown => {}
            HostKeyVerdict::Trusted => verdict = HostKeyVerdict::Trusted,
            changed @ HostKeyVerdict::Changed { .. } => {
                if verdict == HostKeyVerdict::Unknown {
                    verdict = changed;
                }
            }
            revoked @ HostKeyVerdict::Revoked { .. } => return Ok(revoked),
        }
    }
    Ok(verdict)
}

/// Pins `fingerprint` for `host:port`. An existing different pin stays
/// unless `replace` is set (renewing a legitimately changed key).
pub(super) fn pin(host: &str, port: u16, fingerprint: &str, replace: bool) -> io::Result<()> {
    pin_in(&app_data_dir(), host, port, fingerprint, replace)
}

fn pin_in(
    directory: &Path,
    host: &str,
    port: u16,
    fingerprint: &str,
    replace: bool,
) -> io::Result<()> {
    if !fingerprint.starts_with("SHA256:") || fingerprint.contains(char::is_whitespace) {
        return Err(invalid(
            "host-key fingerprint must be an SHA256: fingerprint",
        ));
    }
    let label = format!("{host}:{port}");
    if label.contains(char::is_whitespace) {
        return Err(invalid("host name must not contain whitespace"));
    }
    let _lock = lock_store(directory)?;
    let path = directory.join(STORE_NAME);
    let entries = load_known_hosts(&path)?;
    match entries.iter().find(|(host, _)| host == &label) {
        Some((_, existing)) if existing == fingerprint => Ok(()),
        Some(_) if !replace => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "{label} already has a different pinned host key; confirm the new one with \
                 `se connections trust-host-key {label} --replace` or forget the old one first"
            ),
        )),
        Some(_) => {
            let mut kept: Vec<(String, String)> = entries
                .into_iter()
                .filter(|(host, _)| host != &label)
                .collect();
            kept.push((label, fingerprint.to_string()));
            rewrite(&path, &kept)
        }
        None => append(&path, &label, fingerprint),
    }
}

/// Removes the pin of `host:port`; `Ok(false)` when there was none.
pub(super) fn forget(host: &str, port: u16) -> io::Result<bool> {
    forget_in(&app_data_dir(), host, port)
}

fn forget_in(directory: &Path, host: &str, port: u16) -> io::Result<bool> {
    let label = format!("{host}:{port}");
    let _lock = lock_store(directory)?;
    let path = directory.join(STORE_NAME);
    let entries = load_known_hosts(&path)?;
    let before = entries.len();
    let kept: Vec<(String, String)> = entries
        .into_iter()
        .filter(|(host, _)| host != &label)
        .collect();
    if kept.len() == before {
        return Ok(false);
    }
    rewrite(&path, &kept)?;
    Ok(true)
}

/// Whether Smart Explorer pinned a key for `host:port` (a server known
/// before SHA-1 signatures were dropped may still use them).
pub(super) fn has_pin(host: &str, port: u16) -> bool {
    has_pin_in(&app_data_dir(), host, port)
}

fn has_pin_in(directory: &Path, host: &str, port: u16) -> bool {
    pinned_fingerprint(&directory.join(STORE_NAME), &format!("{host}:{port}"))
        .ok()
        .flatten()
        .is_some()
}

fn pinned_fingerprint(path: &Path, label: &str) -> io::Result<Option<String>> {
    Ok(load_known_hosts(path)?
        .into_iter()
        .find(|(host, _)| host == label)
        .map(|(_, fingerprint)| fingerprint))
}

/// OpenSSH's user and system files, as `ssh` reads them by default.
fn openssh_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Some(home) = crate::support_dirs::home_dir() {
        files.push(home.join(".ssh").join("known_hosts"));
    }
    #[cfg(windows)]
    {
        if let Some(program_data) = std::env::var_os("PROGRAMDATA") {
            files.push(
                PathBuf::from(program_data)
                    .join("ssh")
                    .join("ssh_known_hosts"),
            );
        }
    }
    #[cfg(not(windows))]
    {
        files.push(PathBuf::from("/etc/ssh/ssh_known_hosts"));
    }
    files
}

fn openssh_verdict(
    path: &Path,
    name: &str,
    key: &russh::keys::PublicKey,
) -> io::Result<HostKeyVerdict> {
    let text = match read_limited(path) {
        Ok(Some(text)) => text,
        Ok(None) => return Ok(HostKeyVerdict::Unknown),
        // An unreadable OpenSSH file only means it cannot vouch for a key.
        Err(_) => return Ok(HostKeyVerdict::Unknown),
    };
    let source = path.display().to_string();
    let mut verdict = HostKeyVerdict::Unknown;
    for line in text.lines() {
        let Some(entry) = parse_known_hosts_line(line) else {
            continue;
        };
        if !patterns_match(entry.patterns, name) {
            continue;
        }
        let Ok(recorded) = russh::keys::parse_public_key_base64(entry.key_base64) else {
            continue;
        };
        let same_key = recorded.key_data() == key.key_data();
        if entry.revoked {
            if same_key {
                return Ok(HostKeyVerdict::Revoked { source });
            }
            continue;
        }
        if same_key {
            verdict = HostKeyVerdict::Trusted;
        } else if recorded.algorithm() == key.algorithm() && verdict != HostKeyVerdict::Trusted {
            verdict = HostKeyVerdict::Changed {
                source: source.clone(),
            };
        }
    }
    Ok(verdict)
}

fn read_limited(path: &Path) -> io::Result<Option<String>> {
    // `ssh` follows a link such as a dotfiles-managed `~/.ssh/known_hosts`;
    // anything that is not a regular file (opening a FIFO without a writer
    // would block for good) cannot vouch for a key.
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !file.metadata()?.file_type().is_file() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.take(MAX_KNOWN_HOSTS_BYTES).read_to_end(&mut bytes)?;
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

fn lock_store(directory: &Path) -> io::Result<File> {
    std::fs::create_dir_all(directory)?;
    let lock_path = directory.join(LOCK_NAME);
    reject_non_regular_if_present(&lock_path, "SFTP host-key lock")?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.lock()?;
    Ok(lock)
}

fn append(path: &Path, label: &str, fingerprint: &str) -> io::Result<()> {
    reject_non_regular_if_present(path, "SFTP known-hosts file")?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    validate_regular_file(&file, "SFTP known-hosts file")?;
    writeln!(file, "{label} {fingerprint}")?;
    file.flush()?;
    file.sync_all()
}

fn rewrite(path: &Path, entries: &[(String, String)]) -> io::Result<()> {
    reject_non_regular_if_present(path, "SFTP known-hosts file")?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let staged = parent.join(format!("{STORE_NAME}.new"));
    match std::fs::remove_file(&staged) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)?;
    for (host, fingerprint) in entries {
        writeln!(file, "{host} {fingerprint}")?;
    }
    file.sync_all()?;
    drop(file);
    std::fs::rename(&staged, path)
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
#[path = "known_hosts_tests.rs"]
mod tests;
