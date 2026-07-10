use sha2::Digest;
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::Ordering;

use super::retention::{compare_group, retain_best};
use super::types::{
    ContentHash, DuplicateEvidence, DuplicateGroup, FileCandidate, HashAlgorithm,
    ReclaimConfidence, ReclaimItem, ReclaimOptions, ReclaimProgress,
};
use super::util::{hex_lower, push_bounded_error, to_fwd};

pub(crate) struct DuplicateAnalysis {
    pub(crate) groups: Vec<DuplicateGroup>,
    pub(crate) total_groups: u64,
}

pub(crate) fn duplicate_groups(
    files: Vec<FileCandidate>,
    p: &ReclaimProgress,
    opts: &ReclaimOptions,
    errors: &mut Vec<String>,
    suppressed_errors: &mut u64,
) -> DuplicateAnalysis {
    let mut retained = Vec::with_capacity(opts.max_items.min(files.len()));
    for file in files
        .into_iter()
        .filter(|file| file.item.size >= opts.duplicate_min_bytes)
    {
        retain_best(&mut retained, file, opts.max_items, |left, right| {
            right
                .item
                .size
                .cmp(&left.item.size)
                .then_with(|| left.item.path.cmp(&right.item.path))
        });
    }
    retained.sort_by(|left, right| {
        right
            .item
            .size
            .cmp(&left.item.size)
            .then_with(|| left.item.path.cmp(&right.item.path))
    });
    let mut by_size: BTreeMap<u64, Vec<FileCandidate>> = BTreeMap::new();
    for f in retained {
        by_size.entry(f.item.size).or_default().push(f);
    }

    let mut groups = Vec::new();
    let mut total_groups = 0u64;
    for (size, same_size) in by_size.into_iter().rev().filter(|(_, v)| v.len() > 1) {
        if p.cancel.load(Ordering::Relaxed) {
            break;
        }
        let mut by_fp: BTreeMap<String, Vec<FileCandidate>> = BTreeMap::new();
        for f in same_size {
            if p.cancel.load(Ordering::Relaxed) {
                break;
            }
            match partial_fingerprint(&f.path, f.item.size, opts.partial_fingerprint_bytes, p) {
                Ok(Some(fp)) => {
                    p.fingerprinted.fetch_add(1, Ordering::Relaxed);
                    by_fp.entry(fp).or_default().push(f);
                }
                Ok(None) => break,
                Err(e) => push_bounded_error(
                    errors,
                    suppressed_errors,
                    format!("Fingerprint {}: {}", to_fwd(&f.path), e),
                ),
            }
        }

        for same_fp in by_fp.into_values().filter(|v| v.len() > 1) {
            if p.cancel.load(Ordering::Relaxed) {
                break;
            }
            let mut by_hash: BTreeMap<String, Vec<ReclaimItem>> = BTreeMap::new();
            for f in same_fp {
                if p.cancel.load(Ordering::Relaxed) {
                    break;
                }
                match sha256_file(&f.path, p) {
                    Ok(Some(h)) => {
                        p.hashed.fetch_add(1, Ordering::Relaxed);
                        by_hash.entry(h).or_default().push(f.item);
                    }
                    Ok(None) => break,
                    Err(e) => push_bounded_error(
                        errors,
                        suppressed_errors,
                        format!("Hash {}: {}", to_fwd(&f.path), e),
                    ),
                }
            }
            for (sha256, mut items) in by_hash.into_iter().filter(|(_, v)| v.len() > 1) {
                items.sort_by(|left, right| {
                    right
                        .mtime_ms
                        .cmp(&left.mtime_ms)
                        .then_with(|| left.path.cmp(&right.path))
                });
                for item in &mut items {
                    item.confidence = ReclaimConfidence::HashMatch;
                    item.reason = "Duplikat".to_string();
                }
                let reclaimable = size.saturating_mul(items.len().saturating_sub(1) as u64);
                p.candidates
                    .fetch_add(items.len() as u64, Ordering::Relaxed);
                total_groups = total_groups.saturating_add(1);
                retain_best(
                    &mut groups,
                    DuplicateGroup {
                        hash: ContentHash {
                            algorithm: HashAlgorithm::Sha256,
                            hex: sha256,
                        },
                        evidence: DuplicateEvidence::LocalSha256,
                        size,
                        reclaimable,
                        items,
                    },
                    opts.max_items,
                    compare_group,
                );
            }
        }
    }
    groups.sort_by(compare_group);
    DuplicateAnalysis {
        groups,
        total_groups,
    }
}

fn partial_fingerprint(
    path: &Path,
    size: u64,
    bytes: u64,
    p: &ReclaimProgress,
) -> std::io::Result<Option<String>> {
    let mut f = std::fs::File::open(path)?;
    let sample = bytes.max(1).min(size).min(1024 * 1024) as usize;
    let mut hasher = sha2::Sha256::new();
    hasher.update(size.to_be_bytes());
    let mut buf = vec![0u8; sample];
    if sample > 0 {
        if p.cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        f.read_exact(&mut buf)?;
        hasher.update(&buf);
        if size > sample as u64 {
            if p.cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            f.seek(SeekFrom::Start(size - sample as u64))?;
            f.read_exact(&mut buf)?;
            hasher.update(&buf);
        }
    }
    Ok(Some(hex_lower(&hasher.finalize())))
}

fn sha256_file(path: &Path, p: &ReclaimProgress) -> std::io::Result<Option<String>> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        if p.cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(Some(hex_lower(&hasher.finalize())))
}

pub(crate) fn bytes_equal(
    a: &Path,
    b: &Path,
    p: &ReclaimProgress,
) -> std::io::Result<Option<bool>> {
    let ma = std::fs::metadata(a)?;
    let mb = std::fs::metadata(b)?;
    if ma.len() != mb.len() {
        return Ok(Some(false));
    }
    let mut fa = std::fs::File::open(a)?;
    let mut fb = std::fs::File::open(b)?;
    let mut ba = vec![0u8; 1024 * 1024];
    let mut bb = vec![0u8; 1024 * 1024];
    loop {
        if p.cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let na = fa.read(&mut ba)?;
        let nb = fb.read(&mut bb)?;
        if na != nb {
            return Ok(Some(false));
        }
        if na == 0 {
            return Ok(Some(true));
        }
        if ba[..na] != bb[..nb] {
            return Ok(Some(false));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytics::ReclaimOptions;

    fn temp_base(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("{}_{}", name, std::process::id()))
    }

    #[test]
    fn same_size_different_content_is_not_duplicate() {
        let base = temp_base("se_dupe_diff");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("a.bin"), b"aaaa").unwrap();
        std::fs::write(base.join("b.bin"), b"bbbb").unwrap();
        let item_a = ReclaimItem::new("a".into(), "a.bin".into(), 4, 2, false);
        let item_b = ReclaimItem::new("b".into(), "b.bin".into(), 4, 1, false);
        let p = ReclaimProgress::default();
        let opts = ReclaimOptions {
            duplicate_min_bytes: 1,
            partial_fingerprint_bytes: 2,
            ..ReclaimOptions::default()
        };
        let mut errors = Vec::new();
        let mut suppressed_errors = 0;
        let groups = duplicate_groups(
            vec![
                FileCandidate {
                    path: base.join("a.bin"),
                    item: item_a,
                },
                FileCandidate {
                    path: base.join("b.bin"),
                    item: item_b,
                },
            ],
            &p,
            &opts,
            &mut errors,
            &mut suppressed_errors,
        )
        .groups;
        assert!(groups.is_empty());
        assert!(errors.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn same_prefix_suffix_different_middle_is_not_duplicate() {
        let base = temp_base("se_dupe_middle");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("a.bin"), b"aa1111zz").unwrap();
        std::fs::write(base.join("b.bin"), b"aa2222zz").unwrap();
        let p = ReclaimProgress::default();
        let opts = ReclaimOptions {
            duplicate_min_bytes: 1,
            partial_fingerprint_bytes: 2,
            ..ReclaimOptions::default()
        };
        let mut errors = Vec::new();
        let mut suppressed_errors = 0;
        let groups = duplicate_groups(
            vec![
                FileCandidate {
                    path: base.join("a.bin"),
                    item: ReclaimItem::new("a".into(), "a.bin".into(), 8, 2, false),
                },
                FileCandidate {
                    path: base.join("b.bin"),
                    item: ReclaimItem::new("b".into(), "b.bin".into(), 8, 1, false),
                },
            ],
            &p,
            &opts,
            &mut errors,
            &mut suppressed_errors,
        )
        .groups;
        assert!(groups.is_empty());
        assert_eq!(p.hashed.load(Ordering::Relaxed), 2);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn true_duplicates_produce_one_group_with_newest_first() {
        let base = temp_base("se_dupe_same");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("a.bin"), b"same").unwrap();
        std::fs::write(base.join("b.bin"), b"same").unwrap();
        let p = ReclaimProgress::default();
        let opts = ReclaimOptions {
            duplicate_min_bytes: 1,
            partial_fingerprint_bytes: 2,
            ..ReclaimOptions::default()
        };
        let mut errors = Vec::new();
        let mut suppressed_errors = 0;
        let groups = duplicate_groups(
            vec![
                FileCandidate {
                    path: base.join("a.bin"),
                    item: ReclaimItem::new("a".into(), "a.bin".into(), 4, 10, false),
                },
                FileCandidate {
                    path: base.join("b.bin"),
                    item: ReclaimItem::new("b".into(), "b.bin".into(), 4, 5, false),
                },
            ],
            &p,
            &opts,
            &mut errors,
            &mut suppressed_errors,
        )
        .groups;
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].items[0].name, "a.bin");
        assert_eq!(groups[0].hash.algorithm, HashAlgorithm::Sha256);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cancel_stops_before_hashing() {
        let base = temp_base("se_dupe_cancel");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("a.bin"), b"same").unwrap();
        std::fs::write(base.join("b.bin"), b"same").unwrap();
        let p = ReclaimProgress::default();
        p.cancel.store(true, Ordering::Relaxed);
        let opts = ReclaimOptions {
            duplicate_min_bytes: 1,
            ..ReclaimOptions::default()
        };
        let mut errors = Vec::new();
        let mut suppressed_errors = 0;
        let groups = duplicate_groups(
            vec![
                FileCandidate {
                    path: base.join("a.bin"),
                    item: ReclaimItem::new("a".into(), "a.bin".into(), 4, 10, false),
                },
                FileCandidate {
                    path: base.join("b.bin"),
                    item: ReclaimItem::new("b".into(), "b.bin".into(), 4, 5, false),
                },
            ],
            &p,
            &opts,
            &mut errors,
            &mut suppressed_errors,
        )
        .groups;
        assert!(groups.is_empty());
        assert_eq!(p.hashed.load(Ordering::Relaxed), 0);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn duplicate_reducer_caps_inputs_before_building_hash_maps() {
        let base = temp_base("se_dupe_bounded");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let specs: [(&str, &[u8]); 4] = [
            ("small-a.bin", b"same"),
            ("small-b.bin", b"same"),
            ("large-a.bin", b"largest!"),
            ("large-b.bin", b"largest!"),
        ];
        let mut files = Vec::new();
        for (name, content) in specs {
            let path = base.join(name);
            std::fs::write(&path, content).unwrap();
            files.push(FileCandidate {
                path,
                item: ReclaimItem::new(name.into(), name.into(), content.len() as u64, 1, false),
            });
        }
        let opts = ReclaimOptions {
            max_items: 2,
            duplicate_min_bytes: 1,
            partial_fingerprint_bytes: 2,
            ..ReclaimOptions::default()
        };
        let mut errors = Vec::new();
        let mut suppressed = 0;
        let analysis = duplicate_groups(
            files,
            &ReclaimProgress::default(),
            &opts,
            &mut errors,
            &mut suppressed,
        );

        assert_eq!(analysis.total_groups, 1);
        assert_eq!(analysis.groups.len(), 1);
        assert_eq!(analysis.groups[0].size, 8);
        assert_eq!(analysis.groups[0].items.len(), 2);
        assert!(errors.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }
}
