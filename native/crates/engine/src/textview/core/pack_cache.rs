//! Disk side of the language folder: installing a pack without leaving half
//! a pack behind (staged under a dot name, then renamed), the folder
//! fingerprint, and the compiled syntax set kept as an uncompressed syntect
//! dump keyed by the SHA-256 of the added files.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use syntect::parsing::SyntaxSet;

use super::pack_files::{is_syntax_file_name, SyntaxFile};

/// Changes with the dump layout of this module; the two-face version (and
/// with it the built-in set) enters the key through its listing URL.
const CACHE_TAG: &str = "se-textview-syntaxes/1 syntect-5 newlines fancy-regex";
const CACHE_PREFIX: &str = "syntaxes-";
const CACHE_SUFFIX: &str = ".packdump";

/// Hex SHA-256 over the tag, the two-face version and every file (relative path, length, bytes).
pub(crate) fn cache_key(files: &[SyntaxFile]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(CACHE_TAG.as_bytes());
    hasher.update(two_face::acknowledgement::url().as_bytes());
    for file in files {
        hasher.update((file.rel.len() as u64).to_le_bytes());
        hasher.update(file.rel.as_bytes());
        hasher.update((file.bytes.len() as u64).to_le_bytes());
        hasher.update(&file.bytes);
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn cache_path(cache: &Path, key: &str) -> PathBuf {
    cache.join(format!("{CACHE_PREFIX}{key}{CACHE_SUFFIX}"))
}

/// The cached set for `key`; `None` when missing or unreadable.
pub(crate) fn load(cache: &Path, key: &str) -> Option<SyntaxSet> {
    syntect::dumps::from_uncompressed_dump_file(cache_path(cache, key)).ok()
}

/// Keeps `set` for `key` and removes the dumps of other keys. Failures only
/// cost the next start a rebuild, so they are not reported.
pub(crate) fn store(cache: &Path, key: &str, set: &SyntaxSet) {
    if fs::create_dir_all(cache).is_err() {
        return;
    }
    let target = cache_path(cache, key);
    let stage = cache.join(format!(".{}.tmp", unique_suffix()));
    if syntect::dumps::dump_to_uncompressed_file(set, &stage).is_err()
        || fs::rename(&stage, &target).is_err()
    {
        let _ = fs::remove_file(&stage);
        return;
    }
    let Ok(entries) = fs::read_dir(cache) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(CACHE_PREFIX) && name.ends_with(CACHE_SUFFIX) && entry.path() != target
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Names, sizes and modification times of the syntax files in `folder`.
pub(crate) fn fingerprint(folder: &Path) -> String {
    let mut rows = Vec::new();
    walk_fingerprint(folder, "", 0, &mut rows);
    rows.sort();
    rows.join("\n")
}

fn walk_fingerprint(dir: &Path, prefix: &str, depth: usize, rows: &mut Vec<String>) {
    if depth > 9 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let rel = format!("{prefix}/{name}");
        if kind.is_dir() {
            walk_fingerprint(&entry.path(), &rel, depth + 1, rows);
        } else if kind.is_file() && is_syntax_file_name(&name) {
            let (len, modified) = entry
                .metadata()
                .map(|m| {
                    let modified = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_nanos());
                    (m.len(), modified)
                })
                .unwrap_or_default();
            rows.push(format!("{rel}\t{len}\t{modified}"));
        }
    }
}

/// Writes one syntax file as `folder/target` (staged, then renamed).
pub(crate) fn install_file(folder: &Path, target: &str, bytes: &[u8]) -> io::Result<()> {
    let stage = folder.join(format!(".{}.tmp", unique_suffix()));
    let result = fs::write(&stage, bytes).and_then(|()| rename_new(&stage, &folder.join(target)));
    if result.is_err() {
        let _ = fs::remove_file(&stage);
    }
    result
}

/// Copies the syntax files (keeping their relative paths) and the license
/// files of a folder pack to `folder/name` (staged, then renamed).
pub(crate) fn install_folder(
    folder: &Path,
    name: &str,
    files: &[SyntaxFile],
    licenses: &[PathBuf],
) -> io::Result<()> {
    let stage = folder.join(format!(".{}.tmp", unique_suffix()));
    let result = (|| {
        fs::create_dir(&stage)?;
        for file in files {
            let target = stage.join(Path::new(&file.rel));
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&target, &file.bytes)?;
        }
        for license in licenses {
            if let Some(file_name) = license.file_name() {
                fs::copy(license, stage.join(file_name))?;
            }
        }
        rename_new(&stage, &folder.join(name))
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

/// Renames `stage` to `target` unless `target` exists already.
fn rename_new(stage: &Path, target: &Path) -> io::Result<()> {
    if target.symlink_metadata().is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} existiert bereits", target.display()),
        ));
    }
    fs::rename(stage, target)
}

fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "neu-{}-{nanos}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}
