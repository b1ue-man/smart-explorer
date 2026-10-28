//! GUI admission into the transfer list. Every copy entry point builds an
//! engine job (`transfer_route`) and starts it here at once; transfers on one
//! connection share it through its flow, so there is no fixed limit.
use super::prelude::*;
use super::transfer_center::TransferPurpose;
use super::transfer_route::{paste_job, TransferPlace, TransferSelection};
use super::*;
use crate::transfer::{launch_transfer, TransferJob};

/// Selected roots a quick admission looks at on the GUI thread at most.
const QUICK_ADMISSION_ROOTS: usize = 64;

/// A local paste whose selected folders all have free names in the target
/// folder raises no folder decision, so it starts at once instead of waiting
/// for the preview thread and the next frame (M12, 2026-10-28). At most
/// `QUICK_ADMISSION_ROOTS` roots are looked at here (one `lstat` each, two
/// for a folder); more roots, a remote side, a selection with a base, or any
/// doubt go through the preview as before. The folder policy of the job is
/// unchanged: a name taken meanwhile is still decided by the engine's
/// exclusive reservation ("keep both").
fn quick_admission(job: &TransferJob) -> bool {
    if !(job.source.is_local() && job.target.is_local()) {
        return false;
    }
    let crate::transfer::JobItems::Roots { paths, base: None } = &job.items else {
        return false;
    };
    paths.len() <= QUICK_ADMISSION_ROOTS
        && paths
            .iter()
            .all(|path| root_name_free(path, &job.target_dir))
}

/// Whether the selected `root` raises no folder decision in `target_dir`: it
/// is no plain folder (files and links never do), or no entry of its name is
/// in the target folder. Every error is a doubt.
fn root_name_free(root: &str, target_dir: &str) -> bool {
    let native = crate::transfer::walk_listers::native;
    let Ok(metadata) = crate::local_access::symlink_metadata(&native(root)) else {
        return false;
    };
    let link = crate::local_access::metadata_is_link_like(&native(root), &metadata);
    if !metadata.is_dir() || link {
        return true;
    }
    let name = crate::transfer::engine_names::base_name(root);
    if name.is_empty() {
        return false;
    }
    let destination = format!("{}/{name}", target_dir.trim_end_matches('/'));
    match crate::local_access::symlink_metadata(&native(&destination)) {
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        Ok(_) => false,
    }
}

impl App {
    /// Start `job` and announce it ("⇄ Übertragung gestartet: …").
    pub(in crate::app) fn submit_job(&mut self, job: TransferJob) -> bool {
        self.submit_job_for(job, TransferPurpose::Copy)
    }

    pub(in crate::app) fn submit_job_for(
        &mut self,
        mut job: TransferJob,
        purpose: TransferPurpose,
    ) -> bool {
        if matches!(purpose, TransferPurpose::Provide { .. }) {
            job.recoverable = false;
        }
        if super::folder_conflict_ui::requires_preview(&job) && !quick_admission(&job) {
            return match super::folder_conflict_ui::enqueue(job, purpose) {
                Ok(()) => true,
                Err(error) => {
                    self.error_msg = Some(error);
                    false
                }
            };
        }
        self.submit_approved_job_for(job, purpose)
    }

    pub(in crate::app) fn submit_approved_job_for(
        &mut self,
        job: TransferJob,
        purpose: TransferPurpose,
    ) -> bool {
        let consume_cut = job.mode == CopyMode::Move
            && self.clip.as_ref().is_some_and(|clip| {
                clip.cut
                    && clip.selection.items == job.items
                    && clip.selection.filter == job.filter
                    && clip.selection.source.endpoint.same_namespace(&job.source)
            });
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
                if consume_cut {
                    self.clip = None;
                }
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
