//! GUI admission into the transfer list. Every copy entry point builds an
//! engine job (`transfer_route`) and starts it here at once; transfers on one
//! connection share it through its flow, so there is no fixed limit.
use super::prelude::*;
use super::transfer_center::TransferPurpose;
use super::transfer_route::{paste_job, TransferPlace, TransferSelection};
use super::*;
use crate::transfer::{launch_transfer, TransferJob};

impl App {
    /// Start `job` and announce it ("⇄ Übertragung gestartet: …").
    pub(in crate::app) fn submit_job(&mut self, job: TransferJob) -> bool {
        self.submit_job_for(job, TransferPurpose::Copy)
    }

    pub(in crate::app) fn submit_job_for(
        &mut self,
        job: TransferJob,
        purpose: TransferPurpose,
    ) -> bool {
        let announcement = format!(
            "⇄ Übertragung gestartet: {} Element(e) → {}",
            job.items.len(),
            job.target_label
        );
        match self
            .transfer_center
            .submit(job, purpose, &mut launch_transfer)
        {
            Ok(_) => {
                self.notice = Some((announcement, Instant::now()));
                true
            }
            Err(error) => {
                self.error_msg = Some(error);
                false
            }
        }
    }

    /// Paste or drop `selection` into `target_dir` of `target`.
    pub(in crate::app) fn submit_paste(
        &mut self,
        selection: &TransferSelection,
        target: &TransferPlace,
        target_dir: &str,
        mode: CopyMode,
    ) -> bool {
        match paste_job(selection, target, target_dir, mode) {
            Ok(job) => self.submit_job(job),
            Err(error) => {
                self.error_msg = Some(error);
                false
            }
        }
    }

    /// The side the active tab shows: its connection or the local filesystem.
    pub(in crate::app) fn current_place(&self) -> TransferPlace {
        match &self.remote {
            Some(remote) => TransferPlace::remote(remote.backend.clone(), remote.label.clone()),
            None => TransferPlace::local(),
        }
    }

    /// The side and folder tab `index` shows (the active tab's fields live in
    /// `App`), the folder in the spelling its entries use.
    pub(in crate::app) fn tab_place(&self, index: usize) -> Option<(TransferPlace, String)> {
        if index == self.active_tab {
            if self.root_path.is_empty() {
                return None;
            }
            let place = self.current_place();
            let dir = folder_spelling(&place, self.root_prefix());
            return Some((place, dir));
        }
        let tab = self.tabs.get(index)?;
        if tab.root_path.is_empty() {
            return None;
        }
        let place = match &tab.remote {
            Some(remote) => TransferPlace::remote(remote.backend.clone(), remote.label.clone()),
            None => TransferPlace::local(),
        };
        // The scanner's root row owns the spelling of the tab's paths.
        let dir = tab
            .entries
            .iter()
            .find(|entry| entry.depth == 0)
            .map(|entry| entry.path.to_string())
            .unwrap_or_else(|| tab.root_path.clone());
        Some((place.clone(), folder_spelling(&place, dir)))
    }
}

/// A bare drive letter (`C:`) is drive-relative on Windows; a local target
/// folder is always its root (`C:/`).
fn folder_spelling(place: &TransferPlace, dir: String) -> String {
    if place.is_local() {
        ensure_dir_root(&dir)
    } else {
        dir
    }
}
