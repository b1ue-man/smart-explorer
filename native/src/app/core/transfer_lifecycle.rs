use super::*;

type TransferCompletion = Result<(TransferProgress, Vec<String>, bool), ()>;

impl App {
    pub(in crate::app) fn drain_upload(&mut self) {
        let Some(rx) = self.upload_rx.as_ref() else {
            return;
        };
        let mut done: Option<TransferCompletion> = None;
        for _ in 0..16 {
            match rx.try_recv() {
                Ok(TransferMsg::Progress(progress)) => self.transfer_progress = Some(progress),
                Ok(TransferMsg::Done {
                    progress,
                    errors,
                    canceled,
                }) => {
                    done = Some(Ok((progress, errors, canceled)));
                    break;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    done = Some(Err(()));
                    break;
                }
            }
        }
        let Some(done) = done else {
            return;
        };
        let cancel_requested = self
            .transfer_cancel
            .as_ref()
            .is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::Acquire));
        self.upload_rx = None;
        self.transfer_progress = None;
        self.transfer_cancel = None;
        if let Some(worker) = self.transfer_worker.take() {
            let _ = worker.join();
        }
        let Ok((progress, errors, worker_canceled)) = done else {
            self.error_msg = Some("Übertragungs-Thread wurde ohne Ergebnis beendet.".to_string());
            self.rescan();
            return;
        };
        let canceled = worker_canceled || cancel_requested;
        let incomplete = progress.errors > 0 || progress.files_done < progress.files_total;
        if canceled {
            self.notice = Some((
                format!(
                    "⚠ Übertragung abgebrochen · {} vollständig übertragen",
                    progress.files_done
                ),
                std::time::Instant::now(),
            ));
            if progress.errors > 0 {
                let example = errors
                    .first()
                    .map(String::as_str)
                    .unwrap_or("keine Details");
                self.error_msg = Some(format!(
                    "Abgebrochene Übertragung hatte {} Fehler (z. B. {})",
                    progress.errors, example
                ));
            }
        } else if incomplete {
            let example = errors
                .first()
                .map(String::as_str)
                .unwrap_or("keine Details");
            self.error_msg = Some(format!(
                "Übertragung unvollständig: {} vollständig übertragen, {} Fehler (z. B. {})",
                progress.files_done, progress.errors, example
            ));
        } else {
            self.notice = Some((
                format!("✓ {} übertragen", progress.files_done),
                std::time::Instant::now(),
            ));
        }
        if self.remote.is_some() && !self.root_path.is_empty() {
            self.rescan();
        }
    }
}
