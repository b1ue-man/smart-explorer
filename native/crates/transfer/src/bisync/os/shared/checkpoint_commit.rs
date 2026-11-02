//! One checkpoint group commit: flush every target side that holds
//! unconfirmed (`Deferred`) results, then journal the confirmed ones in one
//! frame. A side that cannot be confirmed keeps its results waiting; they
//! never enter the baseline as successful.
use super::checkpoint_journal::Frame;
use super::checkpoint_run::{CheckpointSink, State};
use super::completion::{CompletedAction, CompletedKind};
use super::types::PairSide;

impl CheckpointSink<'_> {
    /// One group commit: everything queued so far, after the actions an
    /// earlier flush could not record yet.
    pub(super) fn flush(&self, state: &mut State) {
        let queued = std::mem::take(&mut self.inbox().queued);
        state.pending.extend(queued);
        if state.pending.is_empty() {
            return;
        }
        if self.lock.id() != self.key.lock_id.to_ascii_lowercase() {
            self.fail(state, "Zwischenstand hält die falsche Paarsperre".into());
            return;
        }
        let mut flush_a = false;
        let mut flush_b = false;
        for result in state.pending.iter().filter(|action| !action.durable) {
            for side in durability_sides(result.kind) {
                match side {
                    PairSide::A => flush_a = true,
                    PairSide::B => flush_b = true,
                }
            }
        }
        let durable_a = self.flush_side(PairSide::A, flush_a, state);
        let durable_b = self.flush_side(PairSide::B, flush_b, state);
        let mut frame = Frame {
            fold_case: self.keys.fold_case,
            ..Frame::default()
        };
        let mut waiting = Vec::new();
        for action in std::mem::take(&mut state.pending) {
            if !action.durable
                && !durability_sides(action.kind)
                    .into_iter()
                    .all(|side| match side {
                        PairSide::A => durable_a,
                        PairSide::B => durable_b,
                    })
            {
                waiting.push(action);
                continue;
            }
            if let Some(entry) = action.baseline_entry() {
                frame.records.push((action.rel.clone(), entry));
            } else {
                match action.kind {
                    CompletedKind::DirCreated { side } => {
                        frame.dirs_add.push(self.directory_key(&action.rel, side));
                    }
                    CompletedKind::DirRemoved { side } => {
                        frame
                            .dirs_remove
                            .push(self.directory_key(&action.rel, side));
                    }
                    _ => {}
                }
            }
        }
        if !frame.is_empty() {
            let result = frame
                .ensure_fits(
                    &state.records,
                    &state.dirs,
                    state.dirs_bytes,
                    super::SyncLimits::for_memory(crate::transfer::physical_memory()),
                )
                .and_then(|bytes| {
                    state.journal.append(&frame)?;
                    frame.apply(&mut state.records, &mut state.dirs)?;
                    state.dirs_bytes = bytes;
                    Ok(())
                });
            if let Err(error) = result {
                self.fail(state, error.to_string());
                // Keep authoritative deltas for another final flush attempt.
                for (rel, entry) in frame.records {
                    waiting.push(CompletedAction {
                        rel,
                        kind: CompletedKind::Copied { from: PairSide::A },
                        src_sig: entry.0,
                        dst_sig: entry.1,
                        durable: true,
                    });
                }
                for rel in frame.dirs_add {
                    waiting.push(CompletedAction {
                        rel,
                        kind: CompletedKind::DirCreated { side: PairSide::A },
                        src_sig: None,
                        dst_sig: None,
                        durable: true,
                    });
                }
                for rel in frame.dirs_remove {
                    waiting.push(CompletedAction {
                        rel,
                        kind: CompletedKind::DirRemoved { side: PairSide::A },
                        src_sig: None,
                        dst_sig: None,
                        durable: true,
                    });
                }
            }
        }
        state.pending = waiting;
        state.last = std::time::Instant::now();
    }

    fn flush_side(&self, side: PairSide, needed: bool, state: &mut State) -> bool {
        if !needed {
            return true;
        }
        let (backend, root) = match side {
            PairSide::A => (self.endpoints.a, self.endpoints.root_a),
            PairSide::B => (self.endpoints.b, self.endpoints.root_b),
        };
        match crate::vfs::sync_filesystem(backend, root) {
            Ok(true) => true,
            Ok(false) => {
                self.fail(
                    state,
                    format!(
                        "Dateien auf {} noch nicht dauerhaft bestätigt",
                        side.label()
                    ),
                );
                false
            }
            Err(error) => {
                self.fail(state, error.to_string());
                false
            }
        }
    }
}

pub(super) fn durability_sides(kind: CompletedKind) -> Vec<PairSide> {
    match kind {
        CompletedKind::Copied { from } => vec![from.other()],
        CompletedKind::Moved { from } => vec![from, from.other()],
        CompletedKind::Deleted { side }
        | CompletedKind::DirCreated { side }
        | CompletedKind::DirRemoved { side } => vec![side],
    }
}
