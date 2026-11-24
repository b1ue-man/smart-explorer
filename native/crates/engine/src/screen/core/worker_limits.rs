//! How long the daemon waits for a host process, in one place (review
//! 2026-11-19, round 11): the worker client (`host_worker_client.rs`) waits
//! with these limits, and the session's release keeper
//! (`host_release_gate.rs`, `Session::hand_releases_over`) derives its own
//! waits from them, so both cannot drift apart. Round 12 (S1): a batch's
//! text goes to the host process in calls of at most
//! `TEXT_CALL_CHARACTERS` characters (`input_calls.rs`), so a handed-over
//! injection ends within one call's time (`INJECTION_CALL_LIMIT`) and the
//! keeper holds every session's input no longer than before round 11.
use crate::screen::core::input_model::InputEvent;
use std::time::Duration;

const FRAME_SECS: u64 = 10;
const INPUT_SECS: u64 = 2;

/// Time for one picture (the first one starts the encoder).
pub const FRAME_TIMEOUT: Duration = Duration::from_secs(FRAME_SECS);
/// Time for one input batch (SCR-M6), the wait for a running picture
/// request included.
pub const INPUT_TIMEOUT: Duration = Duration::from_secs(INPUT_SECS);
/// Extra time per character of text: X11 types a character without a key
/// of its own on a remapped spare key code and waits 20 ms for clients to
/// read the press (`x11_keymap::type_text`).
pub const TEXT_CHARACTER_TIME: Duration = Duration::from_millis(TEXT_CHARACTER_MS);
const TEXT_CHARACTER_MS: u64 = 40;
/// The most characters of text one injection call carries: their time
/// (`TEXT_CHARACTER_TIME` each) fits one input answer time.
pub const TEXT_CALL_CHARACTERS: usize = (INPUT_SECS * 1000 / TEXT_CHARACTER_MS) as usize;
/// How long one release job may take: a host process's release waits for
/// the link a picture request holds, then for its answer.
pub const RELEASE_JOB_LIMIT: Duration = Duration::from_secs(FRAME_SECS + INPUT_SECS);
/// How long the release keeper tries: a release job's limit plus a second
/// answer time for a try after a refusal.
pub const GATE_LIMIT: Duration = Duration::from_secs(FRAME_SECS + 2 * INPUT_SECS);

/// How long a host process may take to answer `events`: a batch with text
/// gets `TEXT_CHARACTER_TIME` more per character (a family window's input
/// too).
pub fn input_limit(events: &[InputEvent]) -> Duration {
    INPUT_TIMEOUT + text_time(events)
}

/// How long the keeper waits for a handed-over injection: it stops before
/// its next call (`input_calls.rs`), so one call remains, with a release
/// job's link wait and at most `TEXT_CALL_CHARACTERS` characters' time.
pub const INJECTION_CALL_LIMIT: Duration = Duration::from_secs(FRAME_SECS + 2 * INPUT_SECS);

fn text_time(events: &[InputEvent]) -> Duration {
    let characters = events
        .iter()
        .map(|event| match event {
            InputEvent::Text { text } => text.chars().count(),
            _ => 0,
        })
        .fold(0usize, usize::saturating_add);
    TEXT_CHARACTER_TIME.saturating_mul(u32::try_from(characters).unwrap_or(u32::MAX))
}
