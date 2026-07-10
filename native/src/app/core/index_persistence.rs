use super::*;

const INDEX_SAVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

impl App {
    /// Start a snapshot save when due. Cloning is deliberate: the worker owns
    /// an immutable generation while watcher changes continue on the UI-owned
    /// index and set `index_dirty` again.
    pub(in crate::app) fn maybe_save_index(&mut self) {
        if !self.index_dirty
            || self.index_building
            || self.index_save_active()
            || self.index_last_saved.elapsed() < INDEX_SAVE_INTERVAL
        {
            return;
        }

        let snapshot = self.folder_index.clone();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let spawn = std::thread::Builder::new()
            .name("folder-index-save".into())
            .spawn(move || -> Result<(), String> {
                // Path resolution creates the app-data directory, so it belongs
                // on the worker too—not merely the file write itself.
                let target = folder_index_path();
                let result = snapshot.save(&target).map_err(|error| error.to_string());
                let _ = tx.send(());
                result
            });

        match spawn {
            Ok(worker) => {
                // Clear only after the worker owns the snapshot. Any watcher
                // mutation after this point re-dirties the newer generation.
                self.index_dirty = false;
                self.index_last_saved = std::time::Instant::now();
                self.index_save_rx = Some(rx);
                self.index_save_worker = Some(worker);
            }
            Err(error) => {
                self.index_save_rx = None;
                self.index_save_worker = None;
                self.index_dirty = true;
                self.index_last_saved = std::time::Instant::now();
                self.error_msg = Some(format!(
                    "Ordnerindex-Speicherworker konnte nicht gestartet werden: {error}"
                ));
            }
        }
    }

    pub(in crate::app) fn drain_index_save(&mut self) {
        let Some(rx) = self.index_save_rx.take() else {
            if self
                .index_save_worker
                .as_ref()
                .is_some_and(|worker| worker.is_finished())
            {
                self.finish_index_save(false);
            }
            return;
        };

        match rx.try_recv() {
            Ok(()) => self.finish_index_save(true),
            Err(crossbeam_channel::TryRecvError::Empty) => {
                if self.index_save_worker.is_some() {
                    self.index_save_rx = Some(rx);
                } else {
                    self.finish_index_save(false);
                }
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.finish_index_save(false);
            }
        }
    }

    pub(in crate::app) fn index_save_active(&self) -> bool {
        self.index_save_rx.is_some() || self.index_save_worker.is_some()
    }

    /// Called during app shutdown. A save worker is always joined—even if it is
    /// still writing—then a newer dirty generation is saved synchronously.
    pub(in crate::app) fn shutdown_index_persistence(&mut self, index_build_active: bool) {
        let rx = self.index_save_rx.take();
        let worker = self.index_save_worker.take();
        if rx.is_some() || worker.is_some() {
            let outcome = join_save_worker(worker);
            let signaled = rx
                .as_ref()
                .is_some_and(|receiver| receiver.try_recv().is_ok());
            let resolution = resolve_save(self.index_dirty, signaled, outcome);
            self.index_dirty = resolution.dirty;
            if let Some(error) = resolution.error {
                eprintln!("Smart Explorer folder-index background save failed: {error}");
            }
        }

        // An index build owns the same persistence target and has no join handle
        // at the app layer. Its cancellation path must finish alone.
        if self.index_dirty && !index_build_active {
            match self.folder_index.save(&folder_index_path()) {
                Ok(()) => self.index_dirty = false,
                Err(error) => {
                    eprintln!("Ordnerindex konnte beim Beenden nicht gespeichert werden: {error}");
                }
            }
        }
    }

    fn finish_index_save(&mut self, signaled: bool) {
        self.index_save_rx = None;
        let outcome = join_save_worker(self.index_save_worker.take());
        let resolution = resolve_save(self.index_dirty, signaled, outcome);
        self.index_dirty = resolution.dirty;
        if let Some(error) = resolution.error {
            self.error_msg = Some(format!(
                "Ordnerindex konnte nicht gespeichert werden: {error}"
            ));
        }
    }
}

enum SaveWorkerOutcome {
    Saved,
    Failed(String),
    Panicked(String),
    Missing,
}

struct SaveResolution {
    dirty: bool,
    error: Option<String>,
}

fn join_save_worker(
    worker: Option<std::thread::JoinHandle<Result<(), String>>>,
) -> SaveWorkerOutcome {
    let Some(worker) = worker else {
        return SaveWorkerOutcome::Missing;
    };
    match worker.join() {
        Ok(Ok(())) => SaveWorkerOutcome::Saved,
        Ok(Err(error)) => SaveWorkerOutcome::Failed(error),
        Err(payload) => SaveWorkerOutcome::Panicked(panic_detail(payload)),
    }
}

fn resolve_save(
    newer_generation_dirty: bool,
    signaled: bool,
    outcome: SaveWorkerOutcome,
) -> SaveResolution {
    match outcome {
        SaveWorkerOutcome::Saved if signaled => SaveResolution {
            dirty: newer_generation_dirty,
            error: None,
        },
        SaveWorkerOutcome::Saved => SaveResolution {
            dirty: true,
            error: Some("Speicherworker wurde ohne Abschlussmeldung beendet".into()),
        },
        SaveWorkerOutcome::Failed(error) => SaveResolution {
            dirty: true,
            error: Some(error),
        },
        SaveWorkerOutcome::Panicked(error) => SaveResolution {
            dirty: true,
            error: Some(format!("Speicherworker ist abgestürzt: {error}")),
        },
        SaveWorkerOutcome::Missing => SaveResolution {
            dirty: true,
            error: Some("Speicherworker-Zustand war unvollständig".into()),
        },
    }
}

fn panic_detail(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unbekannter Panic-Wert".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_snapshot_preserves_only_newer_dirty_generation() {
        let clean = resolve_save(false, true, SaveWorkerOutcome::Saved);
        assert!(!clean.dirty);
        assert!(clean.error.is_none());

        let newer = resolve_save(true, true, SaveWorkerOutcome::Saved);
        assert!(newer.dirty);
        assert!(newer.error.is_none());
    }

    #[test]
    fn failure_disconnect_panic_and_missing_worker_all_redirty() {
        for resolution in [
            resolve_save(false, true, SaveWorkerOutcome::Failed("disk".into())),
            resolve_save(false, false, SaveWorkerOutcome::Saved),
            resolve_save(false, false, SaveWorkerOutcome::Panicked("boom".into())),
            resolve_save(false, false, SaveWorkerOutcome::Missing),
        ] {
            assert!(resolution.dirty);
            assert!(resolution.error.is_some());
        }
    }
}
