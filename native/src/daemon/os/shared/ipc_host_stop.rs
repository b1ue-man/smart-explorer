use super::ShareHostState;

pub(super) fn stop_locked(state: &mut ShareHostState) -> Result<(), String> {
    ensure_can_stop(state.pending_profiles_base.is_some())?;

    // Establish the barrier before asking the service to stop. If delivery is
    // ambiguous, only an explicit RefreshShare request may release it.
    state.suspended = true;
    let service = state.service.take();
    state.running_server.clear();
    state.signal_connected = false;
    state.signal_error = None;
    if let Some(service) = service {
        service.cmd(crate::share::ShareCmd::Stop)?;
    }
    super::ui_events::push(
        &mut state.ui_events,
        crate::share::ShareEvent::Status("Share-Worker getrennt".to_string()),
    );
    Ok(())
}

fn ensure_can_stop(pending_profile_commit: bool) -> Result<(), String> {
    if pending_profile_commit {
        return Err(
            "Share-Worker kann noch nicht gestoppt werden: Ein dauerhafter Profil-Commit wartet auf Wiederholung; Status erneut abrufen und Stop danach wiederholen"
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ensure_can_stop;

    #[test]
    fn stop_refuses_to_strand_a_pending_profile_commit() {
        let error = ensure_can_stop(true).unwrap_err();
        assert!(error.contains("Profil-Commit"));
        assert!(error.contains("Stop danach wiederholen"));
        ensure_can_stop(false).unwrap();
    }
}
