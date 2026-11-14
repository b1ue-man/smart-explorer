//! The RDP setup state that a probe reports (design 9.2), and the one condition
//! „aus zwischen Sitzungen“ (E29, M11). A se-managed GNOME sharing that is
//! switched off between sessions is set up, not a missing listener. The other
//! rules are those of `self_setup.rs` before E29.
use super::platform;
use crate::rdp::core::{HostKind, HostProbe, HostService, SetupState};

/// Reason of a se-managed GNOME desktop sharing that is switched off between
/// sessions (E29). Only a session start switches it on; no probe does. The
/// GNOME adapter sets this reason from the `Status` of `grdctl status`
/// (`linux_os/gnome.rs`, `apply_status`).
pub const OFF_BETWEEN_SESSIONS: &str = "GNOME desktop sharing set up by se is switched off between sessions; a Remote Desktop session switches it on";

/// The one condition „aus zwischen Sitzungen“ (E29, M11): a service that se
/// manages and whose sharing is disabled. The `running` flag is not part of it:
/// it reports that the GNOME user unit is active, which stays true while the
/// sharing is switched off.
pub fn switched_off_between_sessions(service: &HostService) -> bool {
    service.kind == HostKind::Gnome
        && service.managed_credentials
        && service.unavailable_reason.as_deref() == Some(OFF_BETWEEN_SESSIONS)
}

/// A host whose RDP service starts now, or whose se-managed sharing is switched
/// off between sessions, has no missing listener.
fn available_or_switched_off(probe: &HostProbe) -> bool {
    probe.preferred_service().is_some() || probe.services.iter().any(switched_off_between_sessions)
}

/// The setup state a probe v2 reports with `probe` (design 9.2).
pub fn current_state(probe: &HostProbe) -> SetupState {
    if let Some(state) = platform::impossible() {
        return state;
    }
    state_with_marker(probe, super::read_marker().map(|marker| marker.state))
}

/// `current_state` for a stored marker state; a function of its inputs only.
pub(super) fn state_with_marker(probe: &HostProbe, marker: Option<SetupState>) -> SetupState {
    if available_or_switched_off(probe) {
        return match marker {
            Some(SetupState::Done) => SetupState::Done,
            _ => SetupState::NotNeeded,
        };
    }
    match marker {
        Some(SetupState::Done | SetupState::NotNeeded) => {
            SetupState::Failed(format!("no_listener: {}", unavailable_reason(probe)))
        }
        Some(state) => state,
        None => SetupState::Failed("not_set_up".into()),
    }
}

/// The state that `ensure_rdp_host` records for a finished `platform::ensure`:
/// a `Done` without a startable or switched-off service is a missing listener.
pub(super) fn recorded_state(probe: &HostProbe, state: SetupState) -> SetupState {
    match state {
        SetupState::Done if !available_or_switched_off(probe) => {
            SetupState::Failed(format!("no_listener: {}", unavailable_reason(probe)))
        }
        other => other,
    }
}

fn unavailable_reason(probe: &HostProbe) -> String {
    let mut reasons: Vec<String> = probe
        .services
        .iter()
        .filter_map(|service| {
            let reason = service.startable().err()?;
            Some(format!("{}: {reason}", service.id))
        })
        .collect();
    if let Some(reason) = &probe.unavailable_reason {
        reasons.push(reason.clone());
    }
    if reasons.is_empty() {
        "kein startbarer RDP-Dienst".into()
    } else {
        reasons.join("; ")
    }
}

#[cfg(test)]
#[path = "rdp_netz_state_task_tests.rs"]
mod rdp_netz_state_task_tests;
