//! Retention of legacy app-data versions; kept for the stable public API.
use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use crate::vfs::Backend;
use super::types::{Versioning, VersioningScheme};

/// Prune timestamped recovery snapshots without following link-like entries.
/// Every filesystem error is returned so recovery loss is never silent.
pub fn prune_versions(versions: &Path, versioning: &Versioning) -> io::Result<()> {
    let backend = crate::vfs::LocalBackend::new("/");
    let root_text = unicode_path(versions)?;
    let root = match backend.stat(root_text) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if root.is_symlink || !root.is_dir {
        return Err(invalid("versions root is not a real directory"));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let mut snapshots = Vec::new();
    for entry in std::fs::read_dir(versions)? {
        let entry = entry?;
        let Some(timestamp) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u64>().ok())
        else {
            continue;
        };
        snapshots.push((timestamp, entry.path()));
    }
    snapshots.sort_by_key(|(timestamp, _)| std::cmp::Reverse(*timestamp));

    match versioning.scheme {
        VersioningScheme::Days => {
            if versioning.days == 0 {
                return Ok(());
            }
            let cutoff = now.saturating_sub(versioning.days.saturating_mul(86_400));
            for (timestamp, path) in &snapshots {
                if *timestamp < cutoff {
                    remove_snapshot(&backend, path)?;
                }
            }
            Ok(())
        }
        VersioningScheme::Count => {
            if versioning.count == 0 {
                return Ok(());
            }
            for (_, path) in snapshots.iter().skip(versioning.count as usize) {
                remove_snapshot(&backend, path)?;
            }
            Ok(())
        }
        VersioningScheme::Staggered => keep_per_bucket(&backend, &snapshots, now, staggered_bucket),
        VersioningScheme::Gfs => keep_per_bucket(&backend, &snapshots, now, gfs_bucket),
    }
}

fn remove_snapshot(backend: &crate::vfs::LocalBackend, path: &Path) -> io::Result<()> {
    let path_text = unicode_path(path)?;
    let metadata = backend.stat(path_text)?;
    if metadata.is_symlink || !metadata.is_dir {
        return Err(invalid(format!(
            "refusing to prune non-directory recovery entry: {path_text}"
        )));
    }
    crate::vfs::remove_entry(
        backend,
        &crate::vfs::DeleteTarget {
            path: path_text.to_string(),
            id: metadata.id,
            is_dir: true,
            is_symlink: false,
        },
    )
}

fn keep_per_bucket(
    backend: &crate::vfs::LocalBackend,
    snapshots: &[(u64, PathBuf)],
    now: u64,
    bucket: impl Fn(u64, u64) -> Option<String>,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    for (timestamp, path) in snapshots {
        match bucket(*timestamp, now) {
            Some(key) => {
                if !seen.insert(key) {
                    remove_snapshot(backend, path)?;
                }
            }
            None => remove_snapshot(backend, path)?,
        }
    }
    Ok(())
}

fn staggered_bucket(timestamp: u64, now: u64) -> Option<String> {
    let age = now.saturating_sub(timestamp);
    if age < 86_400 {
        Some(format!("s{timestamp}"))
    } else if age < 30 * 86_400 {
        Some(format!("d{}", timestamp / 86_400))
    } else {
        Some(format!("w{}", timestamp / (7 * 86_400)))
    }
}

fn gfs_bucket(timestamp: u64, now: u64) -> Option<String> {
    let age = now.saturating_sub(timestamp);
    if age < 86_400 {
        Some(format!("h{}", timestamp / 3_600))
    } else if age < 7 * 86_400 {
        Some(format!("d{}", timestamp / 86_400))
    } else if age < 28 * 86_400 {
        Some(format!("w{}", timestamp / (7 * 86_400)))
    } else if age < 365 * 86_400 {
        Some(format!("m{}", timestamp / (30 * 86_400)))
    } else {
        None
    }
}

fn unicode_path(path: &Path) -> io::Result<&str> {
    path.to_str().ok_or_else(|| invalid("bisync persistence path is not Unicode"))
}
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
