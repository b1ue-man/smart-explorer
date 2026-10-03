//! Applying authoritative deltas without a full scan or quadratic spelling
//! lookups. A completed spelling replaces only the same planning key.
use std::collections::{BTreeMap, BTreeSet};

use super::keys::KeyPolicy;
use super::types::{Baseline, Sig};

pub(super) struct RecordBook {
    pub baseline: Baseline,
    names: BTreeMap<String, BTreeSet<String>>,
    keys: KeyPolicy,
    text_bytes: u64,
}

impl RecordBook {
    pub fn new(baseline: Baseline, keys: KeyPolicy) -> Self {
        let mut names: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for rel in baseline.keys() {
            names
                .entry(keys.key(rel).into_owned())
                .or_default()
                .insert(rel.clone());
        }
        let text_bytes = baseline
            .keys()
            .fold(0u64, |bytes, rel| bytes.saturating_add(rel.len() as u64));
        Self {
            baseline,
            names,
            keys,
            text_bytes,
        }
    }

    pub fn set_keys(&mut self, keys: KeyPolicy) {
        if self.keys != keys {
            let baseline = std::mem::take(&mut self.baseline);
            *self = Self::new(baseline, keys);
        }
    }

    pub fn record(&mut self, rel: &str, entry: (Option<Sig>, Option<Sig>)) {
        let key = self.keys.key(rel).into_owned();
        if let Some(previous) = self.names.remove(&key) {
            for old in previous {
                self.baseline.remove(&old);
                self.text_bytes = self.text_bytes.saturating_sub(old.len() as u64);
            }
        }
        if entry != (None, None) {
            self.baseline.insert(rel.to_string(), entry);
            self.text_bytes = self.text_bytes.saturating_add(rel.len() as u64);
            self.names.entry(key).or_default().insert(rel.to_string());
        }
    }

    /// An explicitly stale spelling can be forgotten without erasing a
    /// freshly recorded spelling of the same normalized key.
    pub fn forget(&mut self, rel: &str) {
        if self.baseline.remove(rel).is_some() {
            self.text_bytes = self.text_bytes.saturating_sub(rel.len() as u64);
        }
        let key = self.keys.key(rel).into_owned();
        if let Some(names) = self.names.get_mut(&key) {
            names.remove(rel);
            if names.is_empty() {
                self.names.remove(&key);
            }
        }
    }

    pub fn projected(
        &self,
        entries: &[super::plan_types::Record],
        forget: &[String],
    ) -> (u64, u64) {
        let mut count = self.baseline.len() as u64;
        let mut text = self.text_bytes;
        let mut removed = BTreeSet::<&str>::new();
        let mut inserted = BTreeMap::<String, &str>::new();
        for rel in forget {
            if self.baseline.contains_key(rel) && removed.insert(rel) {
                count = count.saturating_sub(1);
                text = text.saturating_sub(rel.len() as u64);
            }
        }
        for (rel, entry) in entries {
            let key = self.keys.key(rel).into_owned();
            if let Some(previous) = self.names.get(&key) {
                for old in previous {
                    if removed.insert(old) {
                        count = count.saturating_sub(1);
                        text = text.saturating_sub(old.len() as u64);
                    }
                }
            }
            if let Some(old) = inserted.remove(&key) {
                count = count.saturating_sub(1);
                text = text.saturating_sub(old.len() as u64);
            }
            if *entry != (None, None) {
                inserted.insert(key, rel);
                count = count.saturating_add(1);
                text = text.saturating_add(rel.len() as u64);
            }
        }
        (count, text)
    }
}
