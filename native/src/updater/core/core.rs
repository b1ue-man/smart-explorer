use std::io::Read;
use std::path::{Path, PathBuf};

use super::config::appdata_dir;

const STAGED_SHA_MARKER: &str = ".sha256-";

pub(super) fn staged_payload_path(prefix: &str, version: &str, sha256: &str) -> PathBuf {
    let safe_version: String = version
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let suffix = std::env::consts::EXE_SUFFIX;
    appdata_dir().join(format!(
        "{}_{}_{}_{}{}{}{}",
        prefix,
        safe_version,
        std::process::id(),
        nanos,
        STAGED_SHA_MARKER,
        sha256.to_ascii_lowercase(),
        suffix
    ))
}

pub(super) fn staged_sha256_from_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let start = name.find(STAGED_SHA_MARKER)? + STAGED_SHA_MARKER.len();
    let token = name.get(start..start + 64)?.to_ascii_lowercase();
    if token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(token)
    } else {
        None
    }
}

pub(super) fn parse_sha256_file(raw: &str, name: &str) -> Result<String, String> {
    let token = raw
        .split_whitespace()
        .next()
        .ok_or_else(|| format!("Hash-Datei {} ist leer", name))?
        .to_ascii_lowercase();
    if token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(token)
    } else {
        Err(format!(
            "Hash-Datei {} enthaelt keinen gueltigen SHA-256",
            name
        ))
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("Hash lesen {}: {}", path.display(), e))?;
    let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("Hash lesen {}: {}", path.display(), e))?;
        if n == 0 {
            break;
        }
        sha2::Digest::update(&mut hasher, &buf[..n]);
    }
    Ok(format!("{:x}", sha2::Digest::finalize(hasher)))
}

pub(super) fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let actual = sha256_file(path)?;
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        let _ = std::fs::remove_file(path);
        Err(format!(
            "Download-Pruefsumme passt nicht fuer {}: erwartet {}, erhalten {}",
            path.display(),
            expected,
            actual
        ))
    }
}

pub(super) fn copy_file_checked(
    src: &Path,
    dest: &Path,
    label: &str,
    expected_sha256: Option<&str>,
) -> Result<(), String> {
    let expected = std::fs::metadata(src)
        .map_err(|e| format!("{} Quelle lesen {}: {}", label, src.display(), e))?
        .len();
    let copied = match std::fs::copy(src, dest) {
        Ok(n) => n,
        Err(e) => {
            let _ = std::fs::remove_file(dest);
            return Err(format!(
                "{} kopieren ({} -> {}): {}",
                label,
                src.display(),
                dest.display(),
                e
            ));
        }
    };
    let actual = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    if copied != expected || actual != expected {
        let _ = std::fs::remove_file(dest);
        return Err(format!(
            "{} unvollstaendig kopiert: {} von {} Bytes",
            label,
            actual.min(copied),
            expected
        ));
    }

    if let Some(expected_hash) = expected_sha256 {
        verify_sha256(dest, expected_hash)?;
    }

    Ok(())
}

pub(super) fn replace_file_with_staged(
    staged: &Path,
    target: &Path,
    label: &str,
    expected_sha256: Option<&str>,
) -> Result<(), String> {
    let pending = unique_sibling(target, "update-pending");
    let old = unique_sibling(target, "update-old");
    let _ = std::fs::remove_file(&pending);
    copy_file_checked(staged, &pending, label, expected_sha256)?;

    match std::fs::rename(target, &old) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::rename(&pending, target).map_err(|e| {
                let _ = std::fs::remove_file(&pending);
                format!("{} einsetzen ({}): {}", label, target.display(), e)
            })?;
            return Ok(());
        }
        Err(e) => {
            let _ = std::fs::remove_file(&pending);
            return Err(format!(
                "{} Ziel sichern ({}): {}",
                label,
                target.display(),
                e
            ));
        }
    }

    if let Err(e) = std::fs::rename(&pending, target) {
        let restore = std::fs::rename(&old, target);
        let _ = std::fs::remove_file(&pending);
        return match restore {
            Ok(()) => Err(format!("{} einsetzen fehlgeschlagen: {}", label, e)),
            Err(restore_err) => Err(format!(
                "{} einsetzen fehlgeschlagen: {}; Rollback fehlgeschlagen: {}",
                label, e, restore_err
            )),
        };
    }
    let _ = std::fs::remove_file(&old);
    Ok(())
}

fn unique_sibling(target: &Path, role: &str) -> PathBuf {
    let name = target
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "smart_explorer".to_string());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    target.with_file_name(format!("{name}.{role}.{}.{}", std::process::id(), nanos))
}

pub(super) fn parse_ver(s: &str) -> (u64, u64, u64) {
    let mut it = s.trim().trim_start_matches('v').split('.');
    let mut next = || -> u64 {
        it.next()
            .and_then(|x| x.trim().parse::<u64>().ok())
            .unwrap_or(0)
    };
    (next(), next(), next())
}

/// True if released `candidate` is strictly newer than `current` (semver-ish).
/// Lets the UI tell apart "newer release -> offer as an update" from "older
/// release -> offer as a rollback".
pub fn is_newer(candidate: &str, current: &str) -> bool {
    parse_ver(candidate) > parse_ver(current)
}
