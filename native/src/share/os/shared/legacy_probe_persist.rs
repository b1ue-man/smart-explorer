//! Durable contact-only publication of a successful authenticated legacy probe.
//! Identity and current profile policy are checked again after peer I/O.
use super::identity::ShareIdentity;
use super::identity_store::with_matching_identity_generation;
use super::legacy_probe::{check_running, PendingLegacyProbe};
use super::profiles::ShareProfiles;
use super::service::ShareService;
use super::types::{DirectAccessState, ShareEvent, ShareStatus};

pub(super) fn validate_before_probe(
    service: &ShareService,
    probe: &PendingLegacyProbe,
) -> Result<(), String> {
    ShareIdentity::with_current_locked(probe.identity.device_name.clone(), |current| {
        with_matching_identity_generation(&probe.identity, current, |_| {
            check_running(service)?;
            let profiles = ShareProfiles::load_checked(service.profile_home.clone())?;
            probe.check_profiles(&profiles)?;
            let state = service.auth.lock().map_err(|_| "Share-State gesperrt")?;
            probe.check_auth(&state)
        })
    })
}

pub(super) fn persist(service: &ShareService, probe: &PendingLegacyProbe) -> Result<(), String> {
    ShareIdentity::with_current_locked(probe.identity.device_name.clone(), |current| {
        with_matching_identity_generation(&probe.identity, current, |_| {
            // Same opportunistic permit as reciprocal persistence. It is
            // acquired only after peer I/O and never queues before withdrawal.
            let transition = service
                .iroh
                .runtime_transition_slot
                .clone()
                .try_acquire_owned()
                .map_err(|_| "Share-Konfiguration wird geaendert".to_string())?;
            let mut state = service.auth.lock().map_err(|_| "Share-State gesperrt")?;
            check_running(service)?;
            probe.check_auth(&state)?;
            let committed =
                ShareProfiles::mutate_persisted(service.profile_home.clone(), |profiles| {
                    check_running(service)?;
                    probe.check_auth(&state)?;
                    probe.check_profiles(profiles)?;
                    let now = super::core::now_secs();
                    let contact = profiles
                        .direct_contacts
                        .iter_mut()
                        .find(|contact| contact.id == probe.contact_id())
                        .ok_or_else(|| "Direktgeraet wurde entfernt".to_string())?;
                    contact.access_state = DirectAccessState::Accepted;
                    contact.accepted_at = Some(now);
                    contact.accepted_public_key = Some(probe.endpoint.presence.public_key.clone());
                    contact.status = ShareStatus::Available;
                    contact.last_seen = Some(now);
                    contact.last_error = None;
                    Ok(())
                })?;
            let canonical = committed
                .direct_contacts
                .iter()
                .find(|contact| contact.id == probe.contact_id())
                .ok_or_else(|| "Verbindlicher Direktkontakt fehlt".to_string())?;
            let contact = state
                .direct_contacts
                .iter_mut()
                .find(|contact| contact.id == probe.contact_id())
                .ok_or_else(|| "Direktgeraet wurde entfernt".to_string())?;
            // Publish only the committed outgoing authority. Keep live routes,
            // pins, signature markers, replay state and every foreign relation.
            contact.access_state = canonical.access_state.clone();
            contact.accepted_at = canonical.accepted_at;
            contact.accepted_public_key = canonical.accepted_public_key.clone();
            contact.status = canonical.status.clone();
            contact.last_seen = contact.last_seen.max(canonical.last_seen);
            contact.last_error = None;
            drop(state);
            drop(transition);
            // The existing daemon consumer reloads the canonical profile and
            // applies ConfigureProfiles with the normal restriction boundary.
            match service
                .iroh
                .ev
                .try_send(ShareEvent::RuntimeProfilesCommitted)
            {
                Ok(()) | Err(crossbeam_channel::TrySendError::Full(_)) => {}
                Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                    return Err("Share-Ereignisempfaenger ist geschlossen".into());
                }
            }
            check_running(service)
        })
    })
}
