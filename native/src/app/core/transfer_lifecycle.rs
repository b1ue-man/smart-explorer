use super::transfer_jobs::{launch_transfer, FinishedTransfer};
use super::*;

impl App {
    /// Collect finished transfers, report each one, start queued requests
    /// into the freed slots and refresh the remote view once.
    pub(in crate::app) fn drain_transfers(&mut self) {
        let finished = self.transfers.poll();
        if finished.is_empty() {
            return;
        }
        for done in finished {
            self.report_transfer(done);
        }
        if let Err(error) = self.transfers.fill(&mut launch_transfer) {
            self.error_msg = Some(error);
        }
        if self.remote.is_some() && !self.root_path.is_empty() {
            self.rescan();
        }
    }

    fn report_transfer(&mut self, done: FinishedTransfer) {
        let Some((progress, errors, worker_canceled)) = done.outcome else {
            self.error_msg = Some("Übertragungs-Thread wurde ohne Ergebnis beendet.".to_string());
            return;
        };
        let canceled = worker_canceled || done.cancel_requested;
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
    }
}
