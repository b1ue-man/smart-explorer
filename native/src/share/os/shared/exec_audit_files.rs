//! The files of the Exec audit log.
//!
//! Two classes, each with its own file and its own rotation, so that what
//! strangers can produce never pushes out what authorized devices did:
//! `Executions` (`exec-audit.log`) holds refused starts, starts and ends of
//! authorized devices; `Rejections` (`exec-audit-rejected.log`) holds the
//! connections that did not get that far (failed authentication, EndpointIds
//! turned away by their penalty, devices without Exec permission). Anyone can
//! mint EndpointIds for free, so only a separate file bounds their share of
//! the history deterministically; throttling would merely slow the eviction.
//! At 4 MiB a file moves to `<name>.1`, replacing the previous one.
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Mutex;

use serde_json::Value;

pub(super) const ROTATE_AT_BYTES: u64 = 4 * 1024 * 1024;

static WRITER: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Class {
    Executions,
    Rejections,
}

impl Class {
    pub(super) const ALL: [Class; 2] = [Class::Executions, Class::Rejections];

    /// Current file and rotated file.
    pub(super) fn names(self) -> (&'static str, &'static str) {
        match self {
            Self::Executions => ("exec-audit.log", "exec-audit.log.1"),
            Self::Rejections => ("exec-audit-rejected.log", "exec-audit-rejected.log.1"),
        }
    }
}

pub(super) fn append(directory: &Path, class: Class, value: &Value) -> io::Result<()> {
    append_rotating(directory, class, value, ROTATE_AT_BYTES)
}

pub(super) fn append_rotating(
    directory: &Path,
    class: Class,
    value: &Value,
    rotate_at: u64,
) -> io::Result<()> {
    let mut line = serde_json::to_string(value).map_err(io::Error::other)?;
    line.retain(|character| character != '\n' && character != '\r');
    line.push('\n');
    let (name, rotated) = class.names();
    let _guard = WRITER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    crate::support_dirs::ensure_private_dir(directory)?;
    let path = directory.join(name);
    let mut file = open_for_append(&path)?;
    let length = file.seek(SeekFrom::End(0))?;
    if length > 0 && length + line.len() as u64 > rotate_at {
        drop(file);
        std::fs::rename(&path, directory.join(rotated))?;
        file = open_for_append(&path)?;
        file.seek(SeekFrom::End(0))?;
    }
    file.write_all(line.as_bytes())?;
    file.flush()
}

fn open_for_append(path: &Path) -> io::Result<File> {
    match crate::creds::private_storage::open_file(path, true) {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match crate::creds::private_storage::create_file(path) {
                Ok(file) => Ok(file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    crate::creds::private_storage::open_file(path, true)
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

/// The last `lines` entries of both classes together, oldest first (by the
/// `at` time of each entry; entries of the same second keep file order).
pub(super) fn tail_in(directory: &Path, lines: usize) -> io::Result<Vec<String>> {
    let mut entries: Vec<(String, String)> = Vec::new();
    for class in Class::ALL {
        let mut own = read_class(directory, class)?;
        let skip = own.len().saturating_sub(lines);
        entries.extend(
            own.split_off(skip)
                .into_iter()
                .map(|line| (time_of(&line), line)),
        );
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let skip = entries.len().saturating_sub(lines);
    Ok(entries
        .split_off(skip)
        .into_iter()
        .map(|entry| entry.1)
        .collect())
}

fn read_class(directory: &Path, class: Class) -> io::Result<Vec<String>> {
    let (name, rotated) = class.names();
    let mut all = Vec::new();
    for name in [rotated, name] {
        let mut file = match crate::creds::private_storage::open_file(&directory.join(name), false)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        (&mut file)
            .take(ROTATE_AT_BYTES * 2)
            .read_to_end(&mut bytes)?;
        // A line cut by a crash must not hide the lines before it.
        all.extend(String::from_utf8_lossy(&bytes).lines().map(str::to_string));
    }
    Ok(all)
}

fn time_of(line: &str) -> String {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|value| value["at"].as_str().map(str::to_string))
        .unwrap_or_default()
}
