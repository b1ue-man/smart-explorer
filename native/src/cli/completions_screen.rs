//! Completion of `se screen` (SCR-M2): the devices whose screen this device
//! can open, by endpoint with name, relation and state, and the ids of the
//! sessions this device hosts for `se screen stop`. Completion never starts
//! the background worker: devices come from the saved profiles, sessions
//! from a running worker only. `--room` takes the rooms of the access
//! completion.
use clap_complete::CompletionCandidate;

use crate::screen::core::registry::SessionView;
use crate::share::ShareProfiles;

fn candidate(value: &str, help: String) -> CompletionCandidate {
    let help = help.replace(['\t', '\r', '\n'], " ");
    CompletionCandidate::new(value).help(Some(clap::builder::StyledStr::from(help)))
}

/// Devices of `se screen probe|bench`.
pub(super) fn device_candidates() -> Vec<CompletionCandidate> {
    ShareProfiles::load_checked(Some(super::share::default_home()))
        .map(|profiles| device_candidates_from(&profiles, crate::share::core_now_secs()))
        .unwrap_or_default()
}

pub(super) fn device_candidates_from(
    profiles: &ShareProfiles,
    now: i64,
) -> Vec<CompletionCandidate> {
    super::screen::device_choices(profiles, now)
        .iter()
        .map(|choice| {
            let state = if choice.online { "online" } else { "offline" };
            let help = match &choice.room {
                Some(room) => format!("{room} / {} · room · {state}", choice.name),
                None => format!("{} · direct · {state}", choice.name),
            };
            candidate(&choice.endpoint, help)
        })
        .collect()
}

/// Session ids for `se screen stop`; none without a running worker.
pub(super) fn session_candidates() -> Vec<CompletionCandidate> {
    crate::daemon::screen_sessions()
        .map(|(_, sessions)| session_candidates_from(&sessions))
        .unwrap_or_default()
}

pub(super) fn session_candidates_from(sessions: &[SessionView]) -> Vec<CompletionCandidate> {
    sessions
        .iter()
        .map(|session| {
            let help = format!(
                "{} · {} · {}",
                session.device_name,
                session.mode.key(),
                session.state.key()
            );
            candidate(&session.id, help)
        })
        .collect()
}

#[cfg(test)]
#[path = "completions_screen_tests.rs"]
mod tests;
