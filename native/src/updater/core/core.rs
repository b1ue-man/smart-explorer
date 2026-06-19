use std::io::Read;
use std::path::{Path, PathBuf};

use super::config::appdata_dir;

pub(super) fn staged_payload_path(prefix: &str, version: &str) -> PathBuf {
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
    appdata_dir().join(format!(
        "{}_{}_{}_{}.exe",
        prefix,
        safe_version,
        std::process::id(),
        nanos
    ))
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
