//! `conn.forgetHostKey`: removes exactly the `host:port` entry from the SFTP
//! trust-on-first-use store (`known_hosts_sftp.txt`) under the same lock file
//! the SFTP client takes, so the next connect stores the new key again.
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::Path;

use serde_json::{json, Value};

use super::args::{io_error, str_arg};
use super::connections::find;
use crate::creds::Protocol;
use crate::mobile::ApiError;

const STORE_NAME: &str = "known_hosts_sftp.txt";
const LOCK_NAME: &str = "known_hosts_sftp.lock";
const MAX_STORE_BYTES: u64 = 1024 * 1024;

pub(super) fn forget(args: &Value) -> Result<Value, ApiError> {
    let connection = find(str_arg(args, "id")?)?;
    if connection.protocol != Protocol::Sftp {
        return Err(ApiError::new(
            "unsupported",
            "Hostschlüssel gibt es nur bei SFTP-Verbindungen.",
        ));
    }
    let key = format!("{}:{}", connection.host, connection.port);
    forget_in(&crate::support_dirs::app_data_dir(), &key)
        .map_err(|error| io_error("Hostschlüssel vergessen", error))?;
    Ok(json!({}))
}

/// Returns whether an entry was removed.
pub(super) fn forget_in(dir: &Path, host_port: &str) -> io::Result<bool> {
    let store = dir.join(STORE_NAME);
    let lock_path = dir.join(LOCK_NAME);
    require_regular_if_present(&lock_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    lock.lock()?;
    match std::fs::symlink_metadata(&store) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => return Err(not_regular()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&store)?
        .take(MAX_STORE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_STORE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "SFTP known-hosts file exceeds its 1 MiB limit",
        ));
    }
    let text = String::from_utf8(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "SFTP known-hosts file is not valid UTF-8",
        )
    })?;
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| line.split_whitespace().next() != Some(host_port))
        .collect();
    if kept.len() == text.lines().count() {
        return Ok(false);
    }
    let staged = dir.join(format!("{STORE_NAME}.forget"));
    let mut body = kept.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    // A leftover from an interrupted run is replaced; never follow a link.
    match std::fs::remove_file(&staged) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)?;
    file.write_all(body.as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&staged, &store)?;
    Ok(true)
}

fn require_regular_if_present(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(()),
        Ok(_) => Err(not_regular()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn not_regular() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "SFTP host-key store is not a regular file",
    )
}
