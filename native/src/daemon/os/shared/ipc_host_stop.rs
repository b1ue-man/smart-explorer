use super::ShareHostState;

pub(in crate::daemon) fn stop_locked(state: &mut ShareHostState) -> Result<(), String> {
    ensure_can_stop(state.pending_profiles_base.is_some())?;

    // Establish the barrier before asking the service to stop. If delivery is
    // ambiguous, only an explicit RefreshShare request may release it.
    state.suspended = true;
    let service = state.service.take();
    super::end_tracked_offers(state);
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

    #[test]
    fn cli_task_worker_stop_ends_tracked_offers() {
        let mut state = super::ShareHostState::new();
        let published = crate::share::DiscoveryEvent::OfferPublished {
            offer_id: "offer".into(),
            target: crate::share::DiscoveryPublishTarget::Direct,
            display_alias: "Laptop".into(),
            discoverable_until: i64::MAX,
        };
        state.discovery_offers.observe(&published);
        super::stop_locked(&mut state).unwrap();
        assert!(state.suspended);
        assert!(state.discovery_offers.offers(0).is_empty());
        assert!(state.ui_events.iter().any(|event| matches!(
            event,
            crate::share::ShareEvent::Discovery(crate::share::DiscoveryEvent::OfferStopped {
                offer_id,
                reason: crate::share::DiscoveryOfferStopReason::WorkerStopped,
            }) if offer_id == "offer"
        )));
    }
}
