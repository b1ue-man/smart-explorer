use super::*;

impl App {
    pub(in crate::app) fn prepare_for_update_apply(&mut self) -> Result<(), String> {
        self.prepare_for_exit(true)
    }

    pub(in crate::app) fn prepare_for_exit(
        &mut self,
        require_recovery: bool,
    ) -> Result<(), String> {
        if self.shutdown_prepared {
            return Ok(());
        }

        let transfer_worker_active = self
            .transfer_worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished());
        let must_keep_session_temp = transfer_worker_active
            || self.upload_rx.is_some()
            || !self.file_open_rx.is_empty()
            || !self.edit_save_rx.is_empty()
            || self.clip_download_rx.is_some()
            || !self.remote_edits.is_empty();
        if let Err(error) = sync_recovery_manifest(&self.remote_edits) {
            if require_recovery && !self.remote_edits.is_empty() {
                return Err(error.to_string());
            }
            eprintln!("Smart Explorer could not synchronize session recovery files: {error}");
        }

        if let Some(handle) = self.scan_handle.take() {
            handle
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        for tab in &mut self.tabs {
            if let Some(handle) = tab.scan_handle.take() {
                handle
                    .cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        if let Some(handle) = self.copy_handle.take() {
            handle
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(cancel) = self.transfer_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Release);
        }
        if let Some(worker) = self.transfer_worker.take() {
            if worker.is_finished() {
                let _ = worker.join();
            }
        }

        let index_worker_active = self.index_building;
        if let Some(cancel) = self.index_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.index_rx = None;
        self.index_building = false;
        self.folder_search_rx = None;
        if let Some(cancel) = self.clip_key_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(cancel) = self.sync_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Release);
        }
        if let Some(cancel) = self.bisync_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Release);
        }
        if let Some(cancel) = self.preview_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Release);
        }
        drop(self.conflict_resolution.take());
        self.conflict_bulk = None;
        self.shutdown_delete_worker();
        self.cancel_analytics_scan();
        self.cancel_reclaim_scan();

        if !must_keep_session_temp {
            cleanup_session_temp();
        }

        self.shutdown_index_persistence(index_worker_active);
        self.watcher = None;
        self.watcher_rx = None;
        self.shutdown_prepared = true;
        Ok(())
    }
}
