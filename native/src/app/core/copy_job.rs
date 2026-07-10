use super::prelude::*;
use super::*;

impl App {
    /// Admit exactly one local copy worker at a time. The closure is invoked
    /// only after the previous worker has reached a terminal channel state, so
    /// callers cannot accidentally orphan its receiver or cancellation handle.
    pub(in crate::app) fn start_copy_job(
        &mut self,
        mode: CopyMode,
        refresh_after: bool,
        start: impl FnOnce(crossbeam_channel::Sender<CopyMsg>) -> CopyHandle,
    ) -> bool {
        if self.copy_handle.is_some() || self.copy_rx.is_some() {
            self.error_msg = Some("Es läuft bereits ein Kopiervorgang.".to_string());
            return false;
        }

        let (tx, rx) = unbounded();
        let handle = start(tx);
        self.copy_handle = Some(handle);
        self.copy_rx = Some(rx);
        self.copy_progress = Some(CopyProgress {
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            elapsed_ms: 0,
            errors: 0,
            canceled: false,
            done: false,
        });
        self.copy_errors.clear();
        self.copy_active_mode = Some(mode);
        self.copy_refresh_after = refresh_after;
        true
    }

    pub(in crate::app) fn cancel_copy_job(&mut self) {
        let requested = if let Some(handle) = &self.copy_handle {
            handle
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
            true
        } else {
            false
        };
        if requested {
            self.notice = Some((
                "Kopiervorgang wird abgebrochen…".to_string(),
                std::time::Instant::now(),
            ));
        }
    }

    /// Clear the admitted worker context and report whether its source view
    /// must be refreshed. The captured mode, never the editable dialog draft,
    /// determines move completion behavior.
    pub(in crate::app) fn finish_copy_job(&mut self) -> bool {
        self.copy_rx = None;
        self.copy_handle = None;
        self.copy_open = false;
        let mode = self.copy_active_mode.take();
        std::mem::take(&mut self.copy_refresh_after) || mode == Some(CopyMode::Move)
    }
}
