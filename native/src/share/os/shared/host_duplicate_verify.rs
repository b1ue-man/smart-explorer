//! Strengthen provider/agent duplicate hints before sending host SHA-256 evidence.
use crate::{
    analytics::{ContentHash, DuplicateEvidence, DuplicateGroup, HashAlgorithm, ReclaimProgress},
    vfs::Backend,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{self, Read},
    sync::atomic::Ordering,
};

pub(in crate::share) fn sha256_groups(
    backend: &dyn Backend,
    groups: Vec<DuplicateGroup>,
    p: &ReclaimProgress,
    errors: &mut Vec<String>,
    suppressed: &mut u64,
) -> io::Result<Vec<DuplicateGroup>> {
    let mut verified = Vec::new();
    let mut buffer = vec![0; 1024 * 1024];
    for group in groups {
        if group.hash.algorithm == HashAlgorithm::Sha256 {
            verified.push(group);
            continue;
        }
        let mut hashes: BTreeMap<String, Vec<crate::analytics::ReclaimItem>> = BTreeMap::new();
        for item in group.items {
            if p.cancel.load(Ordering::Relaxed) {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let result = (|| {
                let before = backend.stat(&item.path)?;
                if before.is_dir || before.is_symlink || before.special || before.size != item.size
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Duplikat ist kein unveränderter regulärer Datenstrom",
                    ));
                }
                let mut read =
                    crate::vfs::open_read_regular(backend, &item.path, item.backend_id.as_deref())?;
                let mut hash = Sha256::new();
                let mut bytes = 0u64;
                loop {
                    if p.cancel.load(Ordering::Relaxed) {
                        return Err(io::ErrorKind::Interrupted.into());
                    }
                    let n = read.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    bytes = bytes.saturating_add(n as u64);
                    if bytes > item.size {
                        break;
                    }
                    hash.update(&buffer[..n]);
                    p.stage.add_bytes(n as u64);
                }
                if bytes != item.size {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Datei während der Duplikatsuche geändert",
                    ));
                }
                let after = backend.stat(&item.path)?;
                if after.is_dir
                    || after.is_symlink
                    || after.special
                    || after.size != before.size
                    || after.mtime_ms != before.mtime_ms
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Datei während der Duplikatsuche geändert",
                    ));
                }
                Ok(format!("{:x}", hash.finalize()))
            })();
            match result {
                Ok(hash) => hashes.entry(hash).or_default().push(item),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
                Err(error) if errors.len() < 64 => errors.push(format!("{}: {error}", item.path)),
                Err(_) => *suppressed = suppressed.saturating_add(1),
            }
        }
        for (hex, items) in hashes {
            if items.len() > 1 {
                verified.push(DuplicateGroup {
                    hash: ContentHash {
                        algorithm: HashAlgorithm::Sha256,
                        hex,
                    },
                    evidence: DuplicateEvidence::LocalSha256,
                    size: group.size,
                    reclaimable: group.size.saturating_mul(items.len() as u64 - 1),
                    items,
                });
            }
        }
    }
    Ok(verified)
}
