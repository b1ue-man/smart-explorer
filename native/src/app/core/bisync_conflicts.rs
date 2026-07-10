use super::prelude::*;
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum ConflictSide {
    A,
    B,
}

impl ConflictSide {
    fn keep_a(self) -> bool {
        self == Self::A
    }

    pub(in crate::app) fn direction(self) -> &'static str {
        match self {
            Self::A => "A → B",
            Self::B => "B → A",
        }
    }
}

pub(in crate::app) struct ConflictBulkRun {
    pub(in crate::app) side: ConflictSide,
    pub(in crate::app) total: usize,
    pub(in crate::app) completed: usize,
}

impl ConflictBulkRun {
    pub(in crate::app) fn new(side: ConflictSide, total: usize) -> Self {
        Self {
            side,
            total,
            completed: 0,
        }
    }
}

struct ConflictResolutionSuccess {
    rel: String,
    signatures: (Option<crate::bisync::Sig>, Option<crate::bisync::Sig>),
}

struct ConflictResolutionFailure {
    canceled: bool,
    message: String,
}

enum ConflictResolutionMessage {
    Phase(crate::bisync::ResolvePhase),
    Finished(Result<ConflictResolutionSuccess, ConflictResolutionFailure>),
}

pub(in crate::app) struct ConflictResolutionTask {
    index: usize,
    pub(in crate::app) rel: String,
    pub(in crate::app) side: ConflictSide,
    pub(in crate::app) phase: crate::bisync::ResolvePhase,
    from_bulk: bool,
    cancel: Arc<AtomicBool>,
    rx: Receiver<ConflictResolutionMessage>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl ConflictResolutionTask {
    fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    pub(in crate::app) fn cancel_requested(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
}

impl Drop for ConflictResolutionTask {
    fn drop(&mut self) {
        self.request_cancel();
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
}

impl App {
    pub(in crate::app) fn start_conflict_resolution(
        &mut self,
        index: usize,
        side: ConflictSide,
        from_bulk: bool,
    ) -> bool {
        if self.conflict_resolution.is_some() {
            self.error_msg = Some("Es läuft bereits eine Konfliktauflösung.".into());
            return false;
        }
        if self.merge.is_some() || self.merge_load_rx.is_some() || self.merge_apply_rx.is_some() {
            self.error_msg = Some(
                "Die Zeilenzusammenführung muss zuerst abgeschlossen oder geschlossen werden."
                    .into(),
            );
            return false;
        }
        let Some(conflict) = self.bisync_conflicts.get(index).cloned() else {
            self.error_msg = Some("Der ausgewählte Konflikt ist nicht mehr vorhanden.".into());
            return false;
        };
        let Some(context) = self.bisync_ctx.as_ref() else {
            self.error_msg = Some("Konfliktlösung: Synchronisationskontext fehlt".into());
            return false;
        };
        let (a, root_a, b, root_b, pair) = (
            context.a.clone(),
            context.root_a.clone(),
            context.b.clone(),
            context.root_b.clone(),
            context.pair.clone(),
        );
        let rel = conflict.rel.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        // A resolution emits at most five phase/terminal messages. Keeping this
        // bounded prevents a stalled UI receiver from becoming a memory sink.
        let (tx, rx) = crossbeam_channel::bounded(8);
        let spawn = std::thread::Builder::new()
            .name("bisync-conflict".into())
            .spawn(move || {
                let result = crate::bisync::resolve_checked(
                    &*a,
                    &root_a,
                    &*b,
                    &root_b,
                    &conflict,
                    side.keep_a(),
                    &pair,
                    &worker_cancel,
                    |phase| {
                        let _ = tx.try_send(ConflictResolutionMessage::Phase(phase));
                    },
                )
                .map(|signatures| ConflictResolutionSuccess {
                    rel: conflict.rel.clone(),
                    signatures,
                })
                .map_err(|error| ConflictResolutionFailure {
                    canceled: error.kind() == std::io::ErrorKind::Interrupted,
                    message: error.to_string(),
                });
                let _ = tx.send(ConflictResolutionMessage::Finished(result));
            });

        match spawn {
            Ok(worker) => {
                self.conflict_resolution = Some(ConflictResolutionTask {
                    index,
                    rel,
                    side,
                    phase: crate::bisync::ResolvePhase::Preparing,
                    from_bulk,
                    cancel,
                    rx,
                    worker: Some(worker),
                });
                true
            }
            Err(error) => {
                self.conflict_bulk = None;
                self.error_msg = Some(format!(
                    "Konfliktauflösung konnte nicht gestartet werden: {error}"
                ));
                false
            }
        }
    }

    pub(in crate::app) fn cancel_conflict_resolution(&mut self) {
        self.conflict_bulk = None;
        if let Some(task) = &self.conflict_resolution {
            task.request_cancel();
            self.notice = Some((
                "Konfliktauflösung wird sicher abgebrochen…".into(),
                std::time::Instant::now(),
            ));
        }
    }

    pub(in crate::app) fn drain_conflict_resolution(&mut self) {
        let mut terminal = None;
        let mut disconnected = false;
        let Some(task) = self.conflict_resolution.as_mut() else {
            return;
        };
        for _ in 0..8 {
            match task.rx.try_recv() {
                Ok(ConflictResolutionMessage::Phase(phase)) => task.phase = phase,
                Ok(ConflictResolutionMessage::Finished(result)) => {
                    terminal = Some(result);
                    break;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if terminal.is_none() && !disconnected {
            return;
        }

        let Some(mut task) = self.conflict_resolution.take() else {
            self.conflict_bulk = None;
            self.error_msg = Some("Konfliktauflösung verlor ihren Worker-Zustand.".into());
            return;
        };
        let panicked = task
            .worker
            .take()
            .is_some_and(|worker| worker.join().is_err());
        if panicked || (disconnected && terminal.is_none()) {
            self.conflict_bulk = None;
            self.error_msg = Some(
                "Konfliktauflösung wurde ohne verlässliches Ergebnis beendet; der Konflikt bleibt offen."
                    .into(),
            );
            return;
        }

        let Some(terminal) = terminal else {
            self.conflict_bulk = None;
            self.error_msg =
                Some("Konfliktauflösung endete ohne verlässliches Abschlussergebnis.".into());
            return;
        };
        match terminal {
            Ok(result) => self.apply_conflict_resolution_result(task, result),
            Err(failure) => {
                self.conflict_bulk = None;
                if failure.canceled {
                    self.notice = Some((
                        format!(
                            "Auflösung von „{}“ abgebrochen; Konflikt bleibt offen",
                            task.rel
                        ),
                        std::time::Instant::now(),
                    ));
                } else {
                    self.error_msg = Some(format!(
                        "Konfliktauflösung für „{}“: {}",
                        task.rel, failure.message
                    ));
                }
            }
        }
    }

    fn apply_conflict_resolution_result(
        &mut self,
        task: ConflictResolutionTask,
        result: ConflictResolutionSuccess,
    ) {
        if result.rel != task.rel {
            self.conflict_bulk = None;
            self.error_msg = Some("Konfliktauflösung lieferte einen unerwarteten Pfad.".into());
            return;
        }
        let Some(context) = self.bisync_ctx.as_mut() else {
            self.conflict_bulk = None;
            self.error_msg = Some(
                "Konflikt wurde im Dateisystem aufgelöst, aber der Synchronisationskontext fehlt; bitte erneut vergleichen."
                    .into(),
            );
            return;
        };
        context
            .baseline
            .insert(result.rel.clone(), result.signatures);
        self.conflict_baseline_dirty = true;

        let index = self
            .bisync_conflicts
            .get(task.index)
            .filter(|conflict| conflict.rel == result.rel)
            .map(|_| task.index)
            .or_else(|| {
                self.bisync_conflicts
                    .iter()
                    .position(|conflict| conflict.rel == result.rel)
            });
        let Some(index) = index else {
            self.conflict_bulk = None;
            self.error_msg = Some(
                "Konflikt wurde aufgelöst, war aber nicht mehr in der angezeigten Liste; bitte erneut vergleichen."
                    .into(),
            );
            return;
        };
        self.bisync_conflicts.swap_remove(index);
        if task.from_bulk {
            if let Some(bulk) = self.conflict_bulk.as_mut() {
                bulk.completed = bulk.completed.saturating_add(1).min(bulk.total);
            }
        }

        let finished_all = self.bisync_conflicts.is_empty();
        let persisted = !finished_all || self.finish_bisync_conflicts();
        if persisted && (!task.from_bulk || self.conflict_bulk.is_none()) {
            self.notice = Some((
                format!("✓ „{}“ mit {} aufgelöst", result.rel, task.side.direction()),
                std::time::Instant::now(),
            ));
        }
    }

    /// Persist an updated conflict baseline before dismissing the dialog. A
    /// failed save keeps the window open and offers an explicit retry.
    pub(in crate::app) fn finish_bisync_conflicts(&mut self) -> bool {
        if self.conflict_resolution.is_some() {
            self.error_msg =
                Some("Die laufende Konfliktauflösung muss zuerst abgeschlossen werden.".into());
            return false;
        }
        if self.conflict_baseline_dirty {
            let Some(context) = &self.bisync_ctx else {
                self.error_msg = Some(
                    "Konfliktstand konnte nicht gespeichert werden: Synchronisationskontext fehlt"
                        .into(),
                );
                return false;
            };
            let path = crate::bisync::baseline_path(&context.pair);
            if let Err(error) = crate::bisync::save_baseline(&path, &context.baseline) {
                self.error_msg = Some(format!(
                    "Konfliktstand konnte nicht gespeichert werden: {error}"
                ));
                return false;
            }
            self.conflict_baseline_dirty = false;
        }
        self.show_bisync_conflicts = false;
        self.conflict_bulk = None;
        if !self.root_path.is_empty() {
            self.rescan();
        }
        true
    }
}
