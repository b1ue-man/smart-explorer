//! Explicit source/destination snapshots used by filtered clipboard paste.
use super::bridge::{self, LocalJob};
use super::{planning, CopyHandle, CopyMsg};
use crate::transfer::{JobItems, Layout, PairItem};
use crate::types::{Conflict, CopyMode};
use crossbeam_channel::Sender;
use std::path::PathBuf;

/// Copy explicit (absolute source, relative destination) pairs into `dest`.
/// Used for the in-app paste of the filter-aware clipboard, where the
/// relative structure was computed at copy time; the set is validated as a
/// whole before anything is created.
pub fn start_copy_pairs(
    pairs: Vec<(String, String)>,
    dest: PathBuf,
    conflict: Conflict,
    tx: Sender<CopyMsg>,
) -> CopyHandle {
    bridge::spawn(tx, move || {
        planning::validate_pair_budget(&pairs)
            .map_err(|error| (dest.display().to_string(), error))?;
        let items = JobItems::Pairs(
            pairs
                .into_iter()
                .map(|(source, rel)| PairItem::new(source, rel))
                .collect(),
        );
        Ok(LocalJob {
            items,
            layout: Layout::Tree,
            filter: None,
            target_dir: super::path_text(&dest)?,
            conflict,
            mode: CopyMode::Copy,
        })
    })
}
