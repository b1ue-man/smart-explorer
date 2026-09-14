use super::{default_home, log, ShareHostState};

pub(super) fn configure_or_restart_locked(state: &mut ShareHostState) -> Result<(), String> {
    if let Some(error) = &state.identity_error {
        return Err(format!("Share-Identitaet nicht verfuegbar: {error}"));
    }
    if let Some(error) = &state.profiles_error {
        return Err(format!("Share-Profile nicht verfuegbar: {error}"));
    }
    let identity = state
        .identity
        .clone()
        .ok_or_else(|| "Share-Identitaet nicht verfuegbar".to_string())?;
    let lan_presence = crate::share::LanSettings::load()
        .map(|settings| settings.presence_enabled)
        .unwrap_or(false);
    let has_direct_peers = state
        .profiles
        .direct_contacts
        .iter()
        .any(|contact| contact.access_state == crate::share::DirectAccessState::Accepted);
    if !share_service_requested(
        state.suspended,
        &state.server,
        state.profiles.auto_connect,
        lan_presence && has_direct_peers,
    ) {
        if let Some(service) = state.service.take() {
            service.cmd(crate::share::ShareCmd::Stop)?;
        }
        state.running_server.clear();
        state.signal_connected = false;
        state.signal_error = None;
        return Ok(());
    }
    if state
        .service
        .as_ref()
        .is_some_and(crate::share::ShareService::reciprocal_repair_in_flight)
    {
        return Ok(());
    }
    let needs_restart = state
        .service
        .as_ref()
        .map(|service| {
            service.identity.node_id != identity.node_id
                || service.identity.device_id != identity.device_id
                || service.identity.device_name != identity.device_name
                || service.identity.direct_lookup_id != identity.direct_lookup_id
                || service.identity.direct_secret() != identity.direct_secret()
                || state.running_server != state.server
        })
        .unwrap_or(true);
    if needs_restart {
        if let Some(service) = state.service.take() {
            service.cmd(crate::share::ShareCmd::Stop)?;
        }
        state.running_server.clear();
        state.signal_connected = false;
        state.signal_error = None;
        match crate::share::ShareService::start_with_profile_home(
            state.server.clone(),
            identity,
            state.profiles.clone(),
            Some(default_home()),
        ) {
            Ok(service) => {
                log("share worker started");
                configure_service(&service, &state.profiles)?;
                state.running_server = state.server.clone();
                state.service = Some(service);
            }
            Err(error) => return Err(format!("Share-Worker Start: {error}")),
        }
    } else if let Some(service) = &state.service {
        configure_service(service, &state.profiles)?;
    }
    Ok(())
}

/// The service runs for a configured server, or without one when local
/// presence can still reach accepted Direct peers.
fn share_service_requested(
    suspended: bool,
    server: &str,
    auto_connect: bool,
    lan_only_possible: bool,
) -> bool {
    !suspended && auto_connect && (!server.trim().is_empty() || lan_only_possible)
}

pub(in crate::daemon) fn stop_service_locked(state: &mut ShareHostState) -> Result<(), String> {
    if let Some(service) = state.service.take() {
        service.cmd(crate::share::ShareCmd::Stop)?;
    }
    state.running_server.clear();
    state.signal_connected = false;
    Ok(())
}

pub(in crate::daemon) fn configure_service(
    service: &crate::share::ShareService,
    profiles: &crate::share::ShareProfiles,
) -> Result<(), String> {
    if service.reciprocal_repair_in_flight() {
        return Ok(());
    }
    service
        .cmd(crate::share::ShareCmd::ConfigureProfiles {
            profiles: Box::new(profiles.clone()),
        })
        .map(|_| ())
}

pub(in crate::daemon) fn reload_committed_profiles(
    state: &mut ShareHostState,
    previous: &crate::share::ShareProfiles,
    preserve_worker_updates: bool,
) -> Result<(), String> {
    let mut canonical = crate::share::ShareProfiles::load_checked(Some(default_home()))?;
    if preserve_worker_updates {
        super::profile_merge::merge_worker_updates(&mut canonical, previous, &state.profiles);
    }
    state.profiles = canonical;
    state.profiles_error = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::share_service_requested;

    #[test]
    fn explicit_stop_barrier_blocks_periodic_auto_connect_reload() {
        assert!(share_service_requested(false, "127.0.0.1:9", true, false));
        assert!(!share_service_requested(true, "127.0.0.1:9", true, false));
        assert!(!share_service_requested(false, "", true, false));
        assert!(!share_service_requested(false, "127.0.0.1:9", false, false));
    }

    #[test]
    fn lan_cleanup_task_lan_only_operation_needs_no_server() {
        assert!(share_service_requested(false, "", true, true));
        assert!(!share_service_requested(true, "", true, true));
        assert!(!share_service_requested(false, "", false, true));
    }
}
