//! Historical C02 state materialization from a completed real Drive seed.
//! Literal side slots, canonical baseline rels and folded dir existence keys
//! are different parts of the old persisted contract.
use crate::bisync::{self, Baseline, Outcome};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct FourMaps {
    files_a: BTreeMap<String, String>,
    files_b: BTreeMap<String, String>,
    dirs_a: BTreeMap<String, String>,
    dirs_b: BTreeMap<String, String>,
}

/// The caller removed only the extra exact-case copies made by the modern
/// seed. Both original signatures and original side spellings stay intact.
pub(super) fn legacy_folded_state(seed: &Outcome, rel_a: &str, rel_b: &str) -> Baseline {
    let path = bisync::baseline_file(seed.state.as_ref().unwrap()).unwrap();
    assert_eq!(bisync::load_baseline(&path).unwrap(), seed.baseline);
    assert!(!path.with_extension("journal").exists());
    let mut baseline = seed.baseline.clone();
    let a = baseline[rel_a].0;
    let b = baseline[rel_b].1;
    assert!(a.is_some() && b.is_some());
    assert_eq!(folded(rel_a), folded(rel_b));
    if rel_a != rel_b {
        baseline.remove(rel_b);
    }
    baseline.insert(rel_a.into(), (a, b));

    let spelling_path = path.with_extension("spellings.json");
    let mut source: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&spelling_path).unwrap()).unwrap();
    // Read the real recorded slots, then serialize exactly the historical
    // four-map schema without any post-upgrade provenance fields.
    source.as_object_mut().unwrap().retain(|field, _| {
        matches!(field.as_str(), "files_a" | "files_b" | "dirs_a" | "dirs_b")
    });
    let mut maps: FourMaps = serde_json::from_value(source).unwrap();
    if rel_a != rel_b {
        maps.files_a.remove(rel_b);
        maps.files_b.remove(rel_a);
        maps.dirs_a.remove(rel_b.rsplit_once('/').unwrap().0);
        maps.dirs_b.remove(rel_a.rsplit_once('/').unwrap().0);
    }
    for map in [
        &mut maps.files_a,
        &mut maps.files_b,
        &mut maps.dirs_a,
        &mut maps.dirs_b,
    ] {
        for (key, rel) in std::mem::take(map) {
            assert_eq!(key, rel, "seed slots must use the current exact policy");
            assert!(map.insert(folded(&rel), rel).is_none());
        }
    }
    assert_eq!(maps.files_a[&folded(rel_a)], rel_a);
    assert_eq!(maps.files_b[&folded(rel_b)], rel_b);

    // plan_dirs records the dirs observed on both sides; Checkpoint::planned
    // stores their folded keys. Implicit file-copy parents need not occur in
    // the modern seed's dir sidecar, which is not historical identity proof.
    let dirs: BTreeSet<String> = maps
        .dirs_a
        .keys()
        .filter(|key| maps.dirs_b.contains_key(*key))
        .cloned()
        .collect();
    for (rel, entry) in &baseline {
        if entry.0.is_some() && entry.1.is_some() {
            let key = folded(rel);
            assert!(maps.files_a.contains_key(&key) && maps.files_b.contains_key(&key));
            for (end, _) in rel.match_indices('/') {
                assert!(dirs.contains(&folded(&rel[..end])));
            }
        }
    }
    assert!(dirs.contains(&folded(rel_a.rsplit_once('/').unwrap().0)));

    std::fs::write(&spelling_path, serde_json::to_vec_pretty(&maps).unwrap()).unwrap();
    let dirs_path = path.with_extension("dirs.json");
    std::fs::write(&dirs_path, serde_json::to_vec(&dirs).unwrap()).unwrap();
    bisync::save_baseline(&path, &baseline).unwrap();
    let stored_maps: FourMaps =
        serde_json::from_slice(&std::fs::read(&spelling_path).unwrap()).unwrap();
    let stored_dirs: BTreeSet<String> =
        serde_json::from_slice(&std::fs::read(&dirs_path).unwrap()).unwrap();
    assert_eq!(stored_maps, maps);
    assert_eq!(stored_dirs, dirs);
    assert_eq!(bisync::load_baseline(&path).unwrap(), baseline);
    baseline
}

fn folded(rel: &str) -> String {
    // These three historical fixtures use ASCII. NFC plus simple uppercase
    // in the actual old KeyPolicy is exactly ASCII uppercase for their names.
    assert!(rel.is_ascii());
    rel.to_ascii_uppercase()
}
