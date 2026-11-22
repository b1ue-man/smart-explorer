//! How long the daemon waits for a host process, in one place (review
//! 2026-11-19, round 11): the worker client (`host_worker_client.rs`) waits
//! with these limits, and the session's release keeper
//! (`host_release_gate.rs`, `Session::hand_releases_over`) derives its own
//! waits from them, so both cannot drift apart.
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
pub const TEXT_CHARACTER_TIME: Duration = Duration::from_millis(40);
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

/// How long one injection job may take: the link wait of a release job plus
/// the text time of its events.
pub fn injection_job_limit(events: &[InputEvent]) -> Duration {
    RELEASE_JOB_LIMIT + text_time(events)
}

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
