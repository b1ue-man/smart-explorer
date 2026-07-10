use std::collections::BTreeSet;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::types::{Baseline, Sig, Versioning, VersioningScheme};
use crate::vfs::Backend;

const BASELINE_MAGIC: &[u8] = b"SEBL\x02";
const MAX_BASELINE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_BASELINE_ENTRIES: usize = 1_000_000;
const MAX_BASELINE_TEXT_BYTES: usize = 128 * 1024 * 1024;

fn app_data_dir() -> PathBuf {
    crate::support_dirs::sync_data_dir()
}

/// Stable ordered identity for a sync pair. Baselines store `(A, B)` and sync
/// direction is side-sensitive, so reversing the roots must never reuse state.
/// Length prefixes avoid delimiter collisions inside legitimate root strings.
pub fn pair_id(root_a: &str, root_b: &str) -> String {
    pair_id_parts([("", root_a), ("", root_b)])
}

pub fn pair_id_for(
    backend_a: &dyn Backend,
    root_a: &str,
    backend_b: &dyn Backend,
    root_b: &str,
) -> String {
    let identity_a = backend_a.state_identity();
    let identity_b = backend_b.state_identity();
    pair_id_parts([(&identity_a, root_a), (&identity_b, root_b)])
}

fn pair_id_parts(parts: [(&str, &str); 2]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    hash_bytes(&mut hash, b"smart-explorer/bisync-pair/v2");
    for (identity, root) in parts {
        hash_bytes(&mut hash, &(identity.len() as u64).to_be_bytes());
        hash_bytes(&mut hash, identity.as_bytes());
        hash_bytes(&mut hash, &(root.len() as u64).to_be_bytes());
        hash_bytes(&mut hash, root.as_bytes());
    }
    format!("{hash:016x}")
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= *byte as u64;
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

pub fn baseline_path(pair: &str) -> PathBuf {
    app_data_dir().join(format!("baseline_{pair}.sebl"))
}

pub fn versions_dir(pair: &str) -> PathBuf {
    app_data_dir().join(format!("versions_{pair}"))
}

pub fn load_baseline(path: &Path) -> io::Result<Baseline> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Baseline::new()),
        Err(error) => return Err(error),
    };
    if file.metadata()?.len() > MAX_BASELINE_BYTES {
        return Err(invalid("bisync baseline exceeds its byte budget"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BASELINE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BASELINE_BYTES {
        return Err(invalid("bisync baseline exceeds its byte budget"));
    }
    if let Some(body) = bytes.strip_prefix(BASELINE_MAGIC) {
        parse_binary(body)
    } else {
        parse_legacy(&bytes)
    }
}

fn parse_binary(mut input: &[u8]) -> io::Result<Baseline> {
    let mut baseline = Baseline::new();
    let mut text_bytes = 0usize;
    while !input.is_empty() {
        if baseline.len() >= MAX_BASELINE_ENTRIES {
            return Err(invalid("bisync baseline exceeds its entry budget"));
        }
        let rel_len = read_u32(&mut input)? as usize;
        if rel_len == 0 {
            return Err(invalid("bisync baseline contains an empty path"));
        }
        text_bytes = text_bytes
            .checked_add(rel_len)
            .filter(|total| *total <= MAX_BASELINE_TEXT_BYTES)
            .ok_or_else(|| invalid("bisync baseline exceeds its path-text budget"))?;
        let rel_bytes = take(&mut input, rel_len)?;
        let rel = std::str::from_utf8(rel_bytes)
            .map_err(|_| invalid("bisync baseline path is not UTF-8"))?
            .to_string();
        validate_rel(&rel)?;
        let left = read_sig(&mut input)?;
        let right = read_sig(&mut input)?;
        if baseline.insert(rel.clone(), (left, right)).is_some() {
            return Err(invalid(format!("duplicate bisync baseline path: {rel}")));
        }
    }
    Ok(baseline)
}

fn parse_legacy(bytes: &[u8]) -> io::Result<Baseline> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| invalid("legacy bisync baseline is not UTF-8"))?;
    let mut baseline = Baseline::new();
    let mut text_bytes = 0usize;
    for (index, line) in text.lines().enumerate() {
        if baseline.len() >= MAX_BASELINE_ENTRIES {
            return Err(invalid("legacy bisync baseline exceeds its entry budget"));
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 3 {
            return Err(invalid(format!(
                "invalid legacy bisync baseline at line {}",
                index + 1
            )));
        }
        validate_rel(fields[0])?;
        text_bytes = text_bytes
            .checked_add(fields[0].len())
            .filter(|total| *total <= MAX_BASELINE_TEXT_BYTES)
            .ok_or_else(|| invalid("legacy baseline exceeds its path-text budget"))?;
        let value = (parse_legacy_sig(fields[1])?, parse_legacy_sig(fields[2])?);
        if baseline.insert(fields[0].to_string(), value).is_some() {
            return Err(invalid(format!(
                "duplicate legacy bisync baseline path: {}",
                fields[0]
            )));
        }
    }
    Ok(baseline)
}

fn parse_legacy_sig(text: &str) -> io::Result<Option<Sig>> {
    if text == "-" {
        return Ok(None);
    }
    let fields: Vec<&str> = text.split(':').collect();
    if !(2..=3).contains(&fields.len()) {
        return Err(invalid("invalid legacy signature field count"));
    }
    Ok(Some(Sig {
        size: fields[0]
            .parse()
            .map_err(|_| invalid("invalid legacy baseline size"))?,
        mtime_ms: fields[1]
            .parse()
            .map_err(|_| invalid("invalid legacy baseline mtime"))?,
        hash: fields
            .get(2)
            .map(|hash| {
                hash.parse()
                    .map_err(|_| invalid("invalid legacy baseline hash"))
            })
            .transpose()?
            .unwrap_or(0),
    }))
}

pub fn save_baseline(path: &Path, baseline: &Baseline) -> io::Result<()> {
    validate_baseline(baseline)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut staged = None;
    for attempt in 0..1000u32 {
        let candidate = path.with_extension(format!(
            "se-baseline-{}-{nonce:x}-{attempt:x}.tmp",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                let result = write_baseline(&mut file, baseline).and_then(|()| file.sync_all());
                if let Err(error) = result {
                    drop(file);
                    let _ = std::fs::remove_file(&candidate);
                    return Err(error);
                }
                staged = Some(candidate);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    let staged = staged.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate bisync baseline staging file",
        )
    })?;
    let staged_text = unicode_path(&staged)?;
    let path_text = unicode_path(path)?;
    let backend = crate::vfs::LocalBackend::new("/");
    let result = crate::vfs::promote_staged_replace(&backend, staged_text, path_text);
    if result.is_err() {
        let _ = std::fs::remove_file(staged);
    }
    result
}

fn validate_baseline(baseline: &Baseline) -> io::Result<()> {
    if baseline.len() > MAX_BASELINE_ENTRIES {
        return Err(invalid("bisync baseline exceeds its entry budget"));
    }
    let mut bytes = BASELINE_MAGIC.len() as u64;
    let mut text_bytes = 0usize;
    for rel in baseline.keys() {
        validate_rel(rel)?;
        let rel_len = u32::try_from(rel.len()).map_err(|_| invalid("baseline path is too long"))?;
        text_bytes = text_bytes
            .checked_add(rel_len as usize)
            .filter(|total| *total <= MAX_BASELINE_TEXT_BYTES)
            .ok_or_else(|| invalid("bisync baseline exceeds its path-text budget"))?;
        bytes = bytes.saturating_add(4 + rel_len as u64 + 2 + 48);
        if bytes > MAX_BASELINE_BYTES {
            return Err(invalid("bisync baseline exceeds its byte budget"));
        }
    }
    Ok(())
}

fn write_baseline(file: &mut std::fs::File, baseline: &Baseline) -> io::Result<()> {
    file.write_all(BASELINE_MAGIC)?;
    for (rel, (left, right)) in baseline {
        file.write_all(&(rel.len() as u32).to_be_bytes())?;
        file.write_all(rel.as_bytes())?;
        write_sig(file, *left)?;
        write_sig(file, *right)?;
    }
    file.flush()
}

fn write_sig(writer: &mut impl Write, sig: Option<Sig>) -> io::Result<()> {
    match sig {
        None => writer.write_all(&[0]),
        Some(sig) => {
            writer.write_all(&[1])?;
            writer.write_all(&sig.size.to_be_bytes())?;
            writer.write_all(&sig.mtime_ms.to_be_bytes())?;
            writer.write_all(&sig.hash.to_be_bytes())
        }
    }
}

fn read_sig(input: &mut &[u8]) -> io::Result<Option<Sig>> {
    match take(input, 1)?[0] {
        0 => Ok(None),
        1 => Ok(Some(Sig {
            size: u64::from_be_bytes(read_array(input)?),
            mtime_ms: i64::from_be_bytes(read_array(input)?),
            hash: u64::from_be_bytes(read_array(input)?),
        })),
        _ => Err(invalid("invalid bisync baseline signature tag")),
    }
}

fn read_u32(input: &mut &[u8]) -> io::Result<u32> {
    Ok(u32::from_be_bytes(read_array(input)?))
}

fn read_array<const N: usize>(input: &mut &[u8]) -> io::Result<[u8; N]> {
    take(input, N)?
        .try_into()
        .map_err(|_| invalid("truncated bisync baseline"))
}

fn take<'a>(input: &mut &'a [u8], count: usize) -> io::Result<&'a [u8]> {
    if input.len() < count {
        return Err(invalid("truncated bisync baseline"));
    }
    let (head, tail) = input.split_at(count);
    *input = tail;
    Ok(head)
}

fn validate_rel(rel: &str) -> io::Result<()> {
    crate::agent_proto::ValidatedRelativePath::parse(rel).map(|_| ())
}

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
    path.to_str()
        .ok_or_else(|| invalid("bisync persistence path is not Unicode"))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
