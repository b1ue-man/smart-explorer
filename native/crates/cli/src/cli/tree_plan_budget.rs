//! The bounded collection budget of one transfer plan: entries, depth and
//! the text the plan keeps in memory.
use super::{metadata_text_bytes, VfsMeta, MAX_TREE_DEPTH};

const MAX_TREE_ENTRIES: u64 = 1_000_000;
const MAX_TREE_TEXT_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Default)]
pub(super) struct PlanBudget {
    entries: u64,
    text_bytes: u64,
}

impl PlanBudget {
    pub(super) fn reserve_entry(
        &mut self,
        source_path: &str,
        destination_path: &str,
        listed: Option<&VfsMeta>,
        depth: usize,
    ) -> Result<u64, String> {
        if depth > MAX_TREE_DEPTH {
            return Err(format!("transfer tree exceeds {MAX_TREE_DEPTH} levels"));
        }
        self.entries = self
            .entries
            .checked_add(1)
            .ok_or_else(|| "transfer entry count overflow".to_string())?;
        let metadata_text = listed.map(metadata_text_bytes).unwrap_or(0);
        self.add_text(
            (source_path.len() as u64)
                .checked_add(destination_path.len() as u64)
                .and_then(|value| value.checked_add(metadata_text))
                .ok_or_else(|| "transfer path text budget overflow".to_string())?,
        )?;
        self.check()?;
        Ok(metadata_text)
    }

    pub(super) fn account_fresh_metadata(
        &mut self,
        fresh: &VfsMeta,
        accounted: u64,
    ) -> Result<(), String> {
        let fresh_text = metadata_text_bytes(fresh);
        if fresh_text > accounted {
            self.add_text(fresh_text - accounted)?;
        }
        self.check()
    }

    pub(super) fn reserve_auxiliary_path(&mut self, path: &str) -> Result<(), String> {
        self.add_text(path.len() as u64)?;
        self.check()
    }

    pub(super) fn reserve_auxiliary_text(&mut self, bytes: u64) -> Result<(), String> {
        self.add_text(bytes)?;
        self.check()
    }

    pub(super) fn ensure_entry_capacity(&self, additional: usize) -> Result<(), String> {
        let additional = u64::try_from(additional)
            .map_err(|_| "transfer child count does not fit its budget".to_string())?;
        if self
            .entries
            .checked_add(additional)
            .is_none_or(|entries| entries > MAX_TREE_ENTRIES)
        {
            Err("transfer tree exceeds its bounded collection budget".to_string())
        } else {
            Ok(())
        }
    }

    fn add_text(&mut self, bytes: u64) -> Result<(), String> {
        self.text_bytes = self
            .text_bytes
            .checked_add(bytes)
            .ok_or_else(|| "transfer path text budget overflow".to_string())?;
        Ok(())
    }

    fn check(&self) -> Result<(), String> {
        if self.entries > MAX_TREE_ENTRIES || self.text_bytes > MAX_TREE_TEXT_BYTES {
            Err("transfer tree exceeds its bounded collection budget".to_string())
        } else {
            Ok(())
        }
    }
}
