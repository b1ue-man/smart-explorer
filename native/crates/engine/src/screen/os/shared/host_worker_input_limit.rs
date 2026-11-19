//! How long the daemon waits for a host process's answer to input, and
//! that this wait stays strict (review 2026-11-19, Gemini 11.3, Gemini 3/11).
use super::INPUT_TIMEOUT;
use crate::screen::core::input_model::InputEvent;
use crate::screen::core::worker_family::FamilyCommand;
use crate::screen::core::worker_wire::WorkerRequest;
use std::time::Duration;

/// How long a host process may take for `events`. Text may take longer than
/// other input: X11 types a character without a key of its own on a
/// remapped spare key code and waits 20 ms for clients to read the press
/// (`x11_keymap::type_text`); a batch with text gets 40 ms more per
/// character (a family window's input too).
pub(super) fn input_limit(events: &[InputEvent]) -> Duration {
    let characters = events
        .iter()
        .map(|event| match event {
            InputEvent::Text { text } => text.chars().count(),
            _ => 0,
        })
        .fold(0usize, usize::saturating_add);
    INPUT_TIMEOUT
        + Duration::from_millis(40).saturating_mul(u32::try_from(characters).unwrap_or(u32::MAX))
}

/// Input waits for its answer within its limit, even a longer one: a lost
/// answer ends the process (`WorkerReader::receive` with `strict`), the
/// picture stream's messages never keep it waiting.
pub(super) fn strict_answer(request: &WorkerRequest) -> bool {
    matches!(
        request,
        WorkerRequest::Input(_)
            | WorkerRequest::Release(_)
            | WorkerRequest::Family(FamilyCommand::Input(_))
    )
}
