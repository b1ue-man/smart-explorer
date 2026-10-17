use crate::share::ShareEvent;

use super::super::ipc_protocol::VaultEventView;

pub(crate) const MAX_SHARE_HOST_UI_EVENTS: usize = 512;

/// Vault events (`VaultEvent`, C7.4) for the clients that poll
/// `Vault::Status` (GUI, se Desktop every 900 ms, S5 H5): the newest
/// `crate::vault::EVENT_LOG` changes of the session register and the
/// number of the last one. They stay out of `ShareEvent`, which older GUIs
/// could not read. Site ID, state, reason and epoch only.
pub(crate) fn vault_events() -> (u64, Vec<VaultEventView>) {
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

pub(crate) fn push(events: &mut Vec<ShareEvent>, event: ShareEvent) {
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
