//! Share commands and status snapshots served to IPC clients.
use std::time::Duration;

use crate::share::{
    DiscoveryCommand, DiscoveryOfferBook, DiscoveryPublishTarget, OfferLookup, ShareCmd,
    ShareCmdResult,
};

use super::ipc_host::{stop::stop_locked, ui_events, ShareHost, ShareHostState};
use super::ipc_protocol::{ShareCommandReply, ShareWorkerSnapshot};

impl ShareHost {
    pub(super) fn send_command(&self, cmd: ShareCmd) -> Result<ShareCommandReply, String> {
        if matches!(
            &cmd,
            crate::share::ShareCmd::EnableExec { .. }
                | crate::share::ShareCmd::DisableExec { .. }
                | crate::share::ShareCmd::ApplyExecGrant { .. }
                | crate::share::ShareCmd::ConfigureProfiles { .. }
        ) {
            return Err("Dieser Share-Befehl erfordert eine dauerhafte Daemon-Mutation".into());
        }
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "Share-Worker State ist gesperrt".to_string())?;
            if let crate::share::ShareCmd::Stop = &cmd {
                return stop_locked(&mut state).map(|()| ShareCommandReply::Applied);
            }
        }
        let publish_target = match &cmd {
            ShareCmd::Discovery(DiscoveryCommand::Publish { target, .. }) => Some(target.clone()),
            _ => None,
        };
        // Checking for a running offer and publishing must not interleave
        // with another client's publish for the same target.
        let _publish_guard = if publish_target.is_some() {
            Some(
                self.discovery_publish_lock
                    .lock()
                    .map_err(|_| "Discovery-Publish-Sperre ist gesperrt".to_string())?,
            )
        } else {
            None
        };
        self.reload_now()?;
        if let Some(target) = &publish_target {
            self.drain_events();
            let state = self
                .state
                .lock()
                .map_err(|_| "Share-Worker State ist gesperrt".to_string())?;
            let now = crate::share::core_now_secs();
            refuse_second_offer(&state.discovery_offers, target, now)?;
        }
        let service = {
            let state = self
                .state
                .lock()
                .map_err(|_| "Share-Worker State ist gesperrt".to_string())?;
            state.service.clone()
        }
        .ok_or_else(|| "Share-Worker ist nicht aktiv".to_string())?;
        match service.cmd(cmd)? {
            ShareCmdResult::DiscoveryOffer(handle) => {
                // The worker queues the offer's state event before it
                // acknowledges the command, so this drain folds it in.
                self.drain_events();
                let state = self
                    .state
                    .lock()
                    .map_err(|_| "Share-Worker State ist gesperrt".to_string())?;
                discovery_offer_reply(&state.discovery_offers, handle.offer_id)
            }
            ShareCmdResult::DiscoveryExchange(handle) => {
                let exchange_id = handle.exchange_id;
                Ok(ShareCommandReply::DiscoveryExchange { exchange_id })
            }
            ShareCmdResult::Applied | ShareCmdResult::ExecGrant(_) => {
                Ok(ShareCommandReply::Applied)
            }
        }
    }

    /// Snapshot for the GUI poll loop; hands over and clears the UI events.
    pub(super) fn drain_for_ui(&self) -> ShareWorkerSnapshot {
        self.snapshot(true)
    }

    /// Snapshot for a terminal client; it sees a copy of the UI events and
    /// leaves them for the GUI.
    pub(super) fn snapshot_for_client(&self) -> ShareWorkerSnapshot {
        self.snapshot(false)
    }

    fn snapshot(&self, drain: bool) -> ShareWorkerSnapshot {
        let should_reload = self
            .state
            .lock()
            .map(|state| state.last_reload.elapsed() >= Duration::from_secs(5))
            .unwrap_or(false);
        if should_reload {
            if let Err(error) = self.reload_now() {
                if let Ok(mut state) = self.state.lock() {
                    ui_events::push(&mut state.ui_events, crate::share::ShareEvent::Error(error));
                }
            }
        }
        self.drain_events();
        match self.state.lock() {
            Ok(mut state) => state_snapshot(&mut state, drain, crate::share::core_now_secs()),
            Err(_) => ShareWorkerSnapshot::default(),
        }
    }
}

fn state_snapshot(state: &mut ShareHostState, drain: bool, now: i64) -> ShareWorkerSnapshot {
    let running = state.service.is_some();
    let (relay_url, candidates) = state
        .service
        .as_ref()
        .map(|service| (service.relay_url(), service.peer_candidates()))
        .unwrap_or_default();
    let events = if drain {
        std::mem::take(&mut state.ui_events)
    } else {
        state.ui_events.clone()
    };
    ShareWorkerSnapshot {
        events,
        profiles: state.profiles.clone(),
        profile_revision: state.profiles.storage_revision.clone(),
        exec_grant_retry: state.exec_retry.clone(),
        pending_direct_requests: Vec::new(),
        running,
        connected: state.signal_connected,
        last_error: state.signal_error.clone(),
        relay_url,
        candidates,
        lan: state.lan_status.clone(),
        discovery_offers: state.discovery_offers.offers(now),
    }
}

/// One running offer per target, whichever client asks (desktop UI, Android
/// or the terminal).
fn refuse_second_offer(
    book: &DiscoveryOfferBook,
    target: &DiscoveryPublishTarget,
    now: i64,
) -> Result<(), String> {
    match book.offer_for_target(target, now) {
        Some(active) => Err(format!(
            "Dieses Ziel ist bereits suchbar (Angebot {}) – zuerst beenden.",
            active.offer_id
        )),
        None => Ok(()),
    }
}

fn discovery_offer_reply(
    book: &DiscoveryOfferBook,
    offer_id: String,
) -> Result<ShareCommandReply, String> {
    match book.lookup(&offer_id) {
        Some(OfferLookup::Active(offer)) => Ok(ShareCommandReply::DiscoveryOffer { offer }),
        Some(OfferLookup::Ended(reason)) => {
            Ok(ShareCommandReply::DiscoveryOfferEnded { offer_id, reason })
        }
        None => Err(format!(
            "Discovery-Angebot {offer_id} wurde ohne Zustandsereignis bestaetigt"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{discovery_offer_reply, refuse_second_offer, state_snapshot, ShareHostState};
    use crate::daemon::ShareCommandReply;
    use crate::share::{
        DiscoveryEvent, DiscoveryOfferBook, DiscoveryOfferStopReason, DiscoveryPublishTarget,
        ShareEvent,
    };

    #[test]
    fn cli_task_share_command_reply_resolves_offer_from_the_book() {
        let mut book = DiscoveryOfferBook::default();
        book.observe(&DiscoveryEvent::OfferPrepared {
            offer_id: "live".into(),
            target: DiscoveryPublishTarget::Direct,
            display_alias: "Laptop".into(),
            discoverable_until: 1_900_000_000,
        });
        book.observe(&DiscoveryEvent::OfferStopped {
            offer_id: "gone".into(),
            reason: DiscoveryOfferStopReason::TransportError,
        });
        match discovery_offer_reply(&book, "live".into()) {
            Ok(ShareCommandReply::DiscoveryOffer { offer }) => {
                assert_eq!(offer.offer_id, "live");
                assert_eq!(offer.discoverable_until, 1_900_000_000);
                assert!(!offer.published);
            }
            other => panic!("unexpected reply: {other:?}"),
        }
        assert!(matches!(
            discovery_offer_reply(&book, "gone".into()),
            Ok(ShareCommandReply::DiscoveryOfferEnded {
                reason: DiscoveryOfferStopReason::TransportError,
                ..
            })
        ));
        assert!(discovery_offer_reply(&book, "unknown".into()).is_err());
    }

    #[test]
    fn cli_task_daemon_refuses_a_second_offer_for_a_target() {
        let mut book = DiscoveryOfferBook::default();
        let room = DiscoveryPublishTarget::Room {
            room_profile_id: "room".into(),
        };
        let direct = DiscoveryPublishTarget::Direct;
        assert!(refuse_second_offer(&book, &direct, 1_000).is_ok());
        book.observe(&DiscoveryEvent::OfferPublished {
            offer_id: "first".into(),
            target: DiscoveryPublishTarget::Direct,
            display_alias: "Laptop".into(),
            discoverable_until: 1_300,
        });
        let refused = refuse_second_offer(&book, &direct, 1_000).unwrap_err();
        assert!(refused.contains("first"));
        assert!(refuse_second_offer(&book, &room, 1_000).is_ok());
        // An offer past its end no longer blocks the target.
        assert!(refuse_second_offer(&book, &direct, 1_300).is_ok());
        book.observe(&DiscoveryEvent::OfferStopped {
            offer_id: "first".into(),
            reason: DiscoveryOfferStopReason::Requested,
        });
        assert!(refuse_second_offer(&book, &direct, 1_000).is_ok());
    }

    #[test]
    fn cli_task_client_snapshot_leaves_the_gui_events() {
        let mut state = ShareHostState::new();
        state.ui_events.push(ShareEvent::Status("for the GUI".into()));
        state.discovery_offers.observe(&DiscoveryEvent::OfferPrepared {
            offer_id: "offer".into(),
            target: DiscoveryPublishTarget::Direct,
            display_alias: "Laptop".into(),
            discoverable_until: 2_000,
        });

        let terminal = state_snapshot(&mut state, false, 1_000);
        assert_eq!(terminal.events.len(), 1);
        assert_eq!(terminal.discovery_offers.len(), 1);
        assert_eq!(state.ui_events.len(), 1);

        let gui = state_snapshot(&mut state, true, 1_000);
        assert!(matches!(
            gui.events.as_slice(),
            [ShareEvent::Status(text)] if text == "for the GUI"
        ));
        assert_eq!(gui.discovery_offers.len(), 1);
        assert!(state.ui_events.is_empty());
        assert!(state_snapshot(&mut state, false, 2_000)
            .discovery_offers
            .is_empty());
    }
}
