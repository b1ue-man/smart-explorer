use crate::share::ShareEvent;

use super::super::ipc_protocol::VaultEventView;

pub const MAX_SHARE_HOST_UI_EVENTS: usize = 512;

/// Vault events (`VaultEvent`, C7.4) for the clients that poll
/// `Vault::Status` (GUI, se Desktop every 900 ms, S5 H5): the newest
/// `crate::vault::EVENT_LOG` changes of the session register and the
/// number of the last one. They stay out of `ShareEvent`, which older GUIs
/// could not read. Site ID, state, reason and epoch only.
pub fn vault_events() -> (u64, Vec<VaultEventView>) {
    let (seq, events) = crate::vault::register().events();
    let views = events
        .into_iter()
        .map(|event| VaultEventView {
            seq: event.seq,
            site: event.site,
            state: event.state.as_str().to_string(),
            reason: event.reason.map(|reason| reason.as_str().to_string()),
            epoch: event.epoch,
        })
        .collect();
    (seq, views)
}

/// Whether `event` goes out to the GUI clients that poll the backlog. G7
/// (review 2026-10-31): `DiscoveryEvent::OfferRights` is newer than the
/// clients of a handover; an older GUI cannot decode a backlog that holds
/// it. Every client reads an offer's rights from the snapshot instead
/// (`ShareWorkerSnapshot::discovery_offers`, field `rights`, which older
/// clients skip as unknown).
pub fn reaches_clients(event: &ShareEvent) -> bool {
    !matches!(
        event,
        ShareEvent::Discovery(crate::share::DiscoveryEvent::OfferRights { .. })
    )
}

pub fn push(events: &mut Vec<ShareEvent>, event: ShareEvent) {
    if !reaches_clients(&event) {
        return;
    }
    if matches!(
        &event,
        ShareEvent::Discovery(crate::share::DiscoveryEvent::DiscoveryList { .. })
    ) {
        events.retain(|queued| {
            !matches!(
                queued,
                ShareEvent::Discovery(crate::share::DiscoveryEvent::DiscoveryList { .. })
            )
        });
    }
    events.push(event);
    let overflow = events.len().saturating_sub(MAX_SHARE_HOST_UI_EVENTS);
    if overflow > 0 {
        events.drain(0..overflow);
    }
}

#[cfg(test)]
#[path = "abgleich_task_ui_events_tests.rs"]
mod abgleich_task_tests;

#[cfg(test)]
mod tests {
    use super::{push, MAX_SHARE_HOST_UI_EVENTS};
    use crate::share::ShareEvent;

    #[test]
    fn every_append_path_keeps_only_the_newest_bounded_events() {
        let mut events = Vec::new();
        for index in 0..MAX_SHARE_HOST_UI_EVENTS * 3 {
            push(&mut events, ShareEvent::Status(index.to_string()));
        }

        assert_eq!(events.len(), MAX_SHARE_HOST_UI_EVENTS);
        assert!(matches!(
            events.first(),
            Some(ShareEvent::Status(value))
                if value == &(MAX_SHARE_HOST_UI_EVENTS * 2).to_string()
        ));
        assert!(matches!(
            events.last(),
            Some(ShareEvent::Status(value))
                if value == &(MAX_SHARE_HOST_UI_EVENTS * 3 - 1).to_string()
        ));
    }
}
