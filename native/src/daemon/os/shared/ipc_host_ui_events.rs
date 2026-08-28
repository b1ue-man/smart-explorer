use crate::share::ShareEvent;

pub(crate) const MAX_SHARE_HOST_UI_EVENTS: usize = 512;

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
