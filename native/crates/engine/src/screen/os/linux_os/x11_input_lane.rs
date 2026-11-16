//! The input endpoint of an X11 source (plan 2026-11-14 „Bildkette“ B-28;
//! BK3b hand-off, merge W2): input never waits behind a capture call. The
//! host process's input thread uses it while the capture holds the worker
//! lock (`host_worker_input::without_source`), as `WindowsInput` does on
//! Windows. It checks what `X11Source::inject_input` checks, from the facts
//! the last capture published (reference generation, captured geometry, a
//! picture exists, the geometry changed since) and the live geometry read on
//! the input's own X connection (`x11_input::area_now`); then it injects
//! with XTest on that connection (`x11_input::inject_bound`). The capture's
//! connection (XShm `GetImage`) is never touched, so neither waits for the
//! other.
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::super::x11_input::{self, Area};
use crate::screen::core::host_backend::ScreenInputSink;
use crate::screen::core::input_model::InputEvent;
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::ControlBlock;

/// What input checks, as the last capture left it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::screen::os) struct InputFacts {
    pub reference: u64,
    pub geometry: Option<Area>,
    pub changed: bool,
    pub image: bool,
}

/// The facts shared between the source and its input endpoint.
#[derive(Default)]
pub(in crate::screen::os) struct SharedFacts(Mutex<InputFacts>);

impl SharedFacts {
    fn lock(&self) -> MutexGuard<'_, InputFacts> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
    pub(in crate::screen::os) fn publish(&self, facts: InputFacts) {
        *self.lock() = facts;
    }
    pub(in crate::screen::os) fn get(&self) -> InputFacts {
        *self.lock()
    }
}

/// The decision of `X11Source::inject_input` on the published facts and the
/// live geometry `current`: `Ok` when the input may go to `current`.
/// `mark_changed` is set when the geometry moved since the capture.
pub(in crate::screen::os) fn admit(
    facts: &InputFacts,
    reference: u64,
    presented: bool,
    current: Area,
) -> Result<(), (ControlBlock, bool)> {
    if reference != facts.reference {
        return Err((ControlBlock::ControlUnavailable, false));
    }
    if facts.geometry != Some(current) {
        return Err((ControlBlock::ControlUnavailable, true));
    }
    if !facts.image || (!presented && facts.changed) {
        return Err((ControlBlock::ControlUnavailable, false));
    }
    Ok(())
}

/// The input endpoint of one X11 source.
pub(in crate::screen::os) struct X11Input {
    pub source: SourceId,
    pub facts: Arc<SharedFacts>,
}

impl ScreenInputSink for X11Input {
    fn legacy_input_invalid(&self) -> bool {
        self.facts.get().changed
    }
    fn inject(
        &self,
        events: &[InputEvent],
        reference_generation: u64,
        presented: bool,
    ) -> Result<(), ControlBlock> {
        let facts = self.facts.get();
        if reference_generation != facts.reference {
            return Err(ControlBlock::ControlUnavailable);
        }
        let current = x11_input::area_now(&self.source)?;
        if let Err((block, changed)) = admit(&facts, reference_generation, presented, current) {
            if changed {
                self.facts.lock().changed = true;
            }
            return Err(block);
        }
        x11_input::inject_bound(&self.source, current, events)
    }
    fn release(&self, events: &[InputEvent]) -> Result<(), ControlBlock> {
        x11_input::release(events)
    }
}

#[cfg(test)]
#[path = "bildkette_w2_input_task_tests.rs"]
mod tests;
