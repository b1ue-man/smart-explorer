//! Input receipts of one host session (`input_receipt.rs`, round 12 rest,
//! Codex 12/15) and the end of a batch's injection when the control goes
//! (`finish_injection`, round 12 S1).
//!
//! The viewer's `INPUT` frames after its `INPUT_RECEIPTS` are numbered from
//! 1; a batch's numbers follow it through the backlog (a merged batch takes
//! the numbers of the batches it absorbed) to the end of its injection. Each
//! number gets one receipt in `CONTROL_STATE.input`: where the session holds
//! a batch back, blocks or refuses it (`report_held`, `report_block`,
//! `error`, all with the deciding batch), when its injection ends
//! (`applied`), when a sink that queues input for later (Android) says its
//! app performed or refused it (`poll_receipts`), and when the control goes
//! or the session ends before its text was typed (`finish_injection`). A
//! viewer without receipts hears of an untyped text rest by `ERROR`
//! `input_stopped`. The numbers and the queued batches are kept for every
//! viewer (without numbers for one that did not ask): the backlog's numbers
//! stay in its order, and an untyped rest is known either way.
use super::*;
use crate::screen::core::host_backend::ScreenInputSink;
use crate::screen::core::input_receipt::{
    stopped_detail, text_characters, InputReceipt, InputResult, INPUT_STOPPED,
};
use std::collections::VecDeque;
use std::sync::Arc;

/// Batches a queueing sink took, waiting for its app.
struct Queued {
    batches: Vec<u64>,
    /// The sink's mark before the batch and with its last call.
    after: u64,
    mark: u64,
    /// Characters of its text.
    text: u64,
}

/// The receipts of the session's viewer.
#[derive(Default)]
pub(in super::super) struct Receipts {
    /// The viewer asked for receipts (`INPUT_RECEIPTS`).
    asked: bool,
    /// `INPUT` frames since the viewer asked.
    counted: u64,
    /// The numbers of the batch being decided (`input`, `start_batch`).
    deciding: Vec<u64>,
    /// The numbers of each backlog entry, in the backlog's order.
    backlog: VecDeque<Vec<u64>>,
    /// The numbers of the batch being injected and the sink's mark before it.
    injecting: Vec<u64>,
    injecting_after: u64,
    queued: VecDeque<Queued>,
}

impl Session<'_> {
    /// `INPUT_RECEIPTS`: on counts from the next `INPUT`; a second on keeps
    /// the count, off forgets every open receipt.
    pub(in super::super) fn input_receipts(&mut self, on: bool) {
        let receipts = &mut self.health.receipts;
        if !on {
            receipts.asked = false;
            receipts.counted = 0;
            receipts.deciding.clear();
            receipts.injecting.clear();
            receipts.backlog.iter_mut().for_each(Vec::clear);
            receipts
                .queued
                .iter_mut()
                .for_each(|queued| queued.batches.clear());
        }
        receipts.asked |= on;
    }

    /// One `INPUT` arrived: its number decides with it.
    pub(in super::super) fn count_input(&mut self) {
        let receipts = &mut self.health.receipts;
        receipts.deciding.clear();
        if receipts.asked {
            receipts.counted = receipts.counted.saturating_add(1);
            receipts.deciding.push(receipts.counted);
        }
    }

    /// The deciding batch joined the backlog's last entry (`merge_motion`).
    pub(in super::super) fn receipts_merged(&mut self) {
        let receipts = &mut self.health.receipts;
        let deciding = std::mem::take(&mut receipts.deciding);
        if let Some(back) = receipts.backlog.back_mut() {
            back.extend(deciding);
        }
    }

    /// The deciding batch waits as the backlog's new last entry; `numbered:
    /// false` for an entry of another batch's releases only.
    pub(in super::super) fn receipts_waiting(&mut self, numbered: bool) {
        let receipts = &mut self.health.receipts;
        let numbers = if numbered {
            std::mem::take(&mut receipts.deciding)
        } else {
            Vec::new()
        };
        receipts.backlog.push_back(numbers);
    }

    /// The backlog's first entry is decided next (`settle_input`).
    pub(in super::super) fn receipts_next(&mut self) {
        let receipts = &mut self.health.receipts;
        receipts.deciding = receipts.backlog.pop_front().unwrap_or_default();
    }

    /// The deciding batch is injected now (`start_batch`).
    pub(in super::super) fn receipts_injecting(&mut self, sink: Option<&Arc<dyn ScreenInputSink>>) {
        let receipts = &mut self.health.receipts;
        receipts.injecting = std::mem::take(&mut receipts.deciding);
        receipts.injecting_after = sink.and_then(|sink| sink.input_mark()).unwrap_or(0);
    }

    /// The deciding batch is not injected: its receipts.
    pub(in super::super) async fn settle_deciding(
        &mut self,
        result: InputResult,
    ) -> io::Result<()> {
        let batches = std::mem::take(&mut self.health.receipts.deciding);
        self.send_receipts(batches, result).await
    }

    /// The numbers of the injection that ended and the sink's mark before it.
    pub(in super::super) fn receipts_ended(&mut self) -> (Vec<u64>, u64) {
        let receipts = &mut self.health.receipts;
        (
            std::mem::take(&mut receipts.injecting),
            receipts.injecting_after,
        )
    }

    /// An injection of `events` went in whole: applied, or for a sink that
    /// queues input for later, waiting for its app (`poll_receipts`).
    pub(in super::super) async fn receipts_applied(
        &mut self,
        (batches, after): (Vec<u64>, u64),
        events: &[InputEvent],
    ) -> io::Result<()> {
        let mark = self
            .pipeline
            .input_sink()
            .and_then(|sink| sink.input_mark());
        let text = text_characters(events);
        match mark {
            // Kept also without numbers when it holds text: an untyped rest
            // is told either way (`finish_injection`).
            Some(mark) if !batches.is_empty() || text > 0 => {
                // A batch merged into one still waiting has that one's mark.
                let after = if mark > after {
                    after
                } else {
                    mark.saturating_sub(1)
                };
                self.health.receipts.queued.push_back(Queued {
                    batches,
                    after,
                    mark,
                    text,
                });
                Ok(())
            }
            Some(_) => Ok(()),
            None => self.send_receipts(batches, InputResult::Applied).await,
        }
    }

    /// Receipts of a sink's queued batches its app finished, in order (on
    /// every viewer frame and capture tick, `settle_input`).
    pub(in super::super) async fn poll_receipts(&mut self) -> io::Result<()> {
        loop {
            let Some(front) = self
                .health
                .receipts
                .queued
                .front()
                .map(|queued| (queued.after, queued.mark, queued.text))
            else {
                return Ok(());
            };
            let (after, mark, text) = front;
            // A source without the sink that took them never says it.
            let result = match self.pipeline.input_sink() {
                Some(sink) => match sink.input_outcome(after, mark) {
                    None => return Ok(()),
                    Some(None) => InputResult::Applied,
                    Some(Some(reason)) => InputResult::Refused { reason },
                },
                None => InputResult::Stopped {
                    untyped: text,
                    exact: false,
                },
            };
            let batches = self
                .health
                .receipts
                .queued
                .pop_front()
                .map(|queued| queued.batches)
                .unwrap_or_default();
            self.send_receipts(batches, result).await?;
        }
    }

    /// The receipts of an injection that ended without being applied.
    pub(in super::super) async fn receipts_settled(
        &mut self,
        batches: Vec<u64>,
        result: InputResult,
    ) -> io::Result<()> {
        self.send_receipts(batches, result).await
    }

    async fn send_receipts(&mut self, batches: Vec<u64>, result: InputResult) -> io::Result<()> {
        for batch in batches {
            let state = ControlState {
                holder: self.handle.controller(),
                blocked: self.health.block_reported,
                held: self.health.held_reported,
                input: Some(InputReceipt {
                    batch,
                    result: result.clone(),
                }),
            };
            self.frame(HostFrame::ControlState(state)).await?;
        }
        Ok(())
    }

    /// Codex 7/10: before the control goes, the batch being injected ends
    /// (at most `CLOSE_GRACE`); its keys count as held then, so the release
    /// that follows lets go of them, and the batches waiting behind it are
    /// dropped (their releases are part of that release). Round 12 rest:
    /// what of their text was not typed reaches the viewer, as receipts or as
    /// `ERROR` `input_stopped`; the viewer may be gone (the session ends), so
    /// a failed write changes nothing here.
    pub(in super::super) async fn finish_injection(&mut self) {
        let mut untyped = 0u64;
        let mut exact = true;
        let mut stopped: Vec<(Vec<u64>, u64, bool)> = Vec::new();
        let backlog = std::mem::take(&mut self.health.backlog);
        let numbers = std::mem::take(&mut self.health.receipts.backlog);
        for (index, entry) in backlog.iter().enumerate() {
            let text = text_characters(entry);
            untyped = untyped.saturating_add(text);
            stopped.push((numbers.get(index).cloned().unwrap_or_default(), text, true));
        }
        // A queueing sink's batches its app has not finished are not
        // performed any more (`ScreenInputSink::revoke`); one may be running.
        let _ = self.poll_receipts().await;
        let queued = std::mem::take(&mut self.health.receipts.queued);
        for queued in queued {
            untyped = untyped.saturating_add(queued.text);
            exact &= queued.text == 0;
            stopped.push((queued.batches, queued.text, false));
        }
        if let Some(mut job) = self.health.injecting.take() {
            // Round 12 (S1): the rest of its calls does not go.
            job.cancel.store(true, std::sync::atomic::Ordering::Release);
            let applied = match tokio::time::timeout(CLOSE_GRACE, &mut job.handle).await {
                Ok(result) => Some(result.map(|result| result.is_ok()).unwrap_or(false)),
                Err(_) => None,
            };
            let numbers = self.receipts_ended();
            match applied {
                None => {
                    super::super::visibility::note_viewer(
                        "an input injection outlasted the close grace",
                    );
                    // Round 8 (I8): it stays the session's, so no release
                    // runs beside it; `hand_releases_over` gives it to the
                    // keeper. Round 9 (E5): only its presses count as held; a
                    // release in it may not have gone, so what it releases
                    // stays held and is let go again (a second release is
                    // harmless).
                    for event in job.events.iter().filter(|event| !releases(event)) {
                        self.pressed.observe(event);
                    }
                    let rest =
                        text_characters(&job.events).saturating_sub(text_characters(job.went()));
                    untyped = untyped.saturating_add(rest);
                    exact &= rest == 0;
                    stopped.push((numbers.0, rest, false));
                    self.health.injecting = Some(job);
                }
                Some(true) => {
                    for event in &job.events {
                        self.pressed.observe(event);
                    }
                    let _ = self.receipts_applied(numbers, &job.events).await;
                    // A queueing sink's app will not perform it any more.
                    let queued = std::mem::take(&mut self.health.receipts.queued);
                    for queued in queued {
                        untyped = untyped.saturating_add(queued.text);
                        exact &= queued.text == 0;
                        stopped.push((queued.batches, queued.text, false));
                    }
                }
                // A stopped or failed batch: the calls that went count (S1).
                Some(false) => {
                    let went: Vec<InputEvent> = job.went().cloned().collect();
                    for event in &went {
                        self.pressed.observe(event);
                    }
                    let rest = text_characters(&job.events).saturating_sub(text_characters(&went));
                    untyped = untyped.saturating_add(rest);
                    stopped.push((numbers.0, rest, true));
                }
            }
        }
        if self.health.receipts.asked {
            for (batches, text, exact) in stopped {
                let result = InputResult::Stopped {
                    untyped: text,
                    exact,
                };
                let _ = self.send_receipts(batches, result).await;
            }
        } else if untyped > 0 {
            let detail = stopped_detail(untyped, exact);
            let error = ScreenWireError {
                code: INPUT_STOPPED.into(),
                detail: Some(detail),
            };
            let _ = self.frame(HostFrame::Error(error)).await;
        }
    }
}
