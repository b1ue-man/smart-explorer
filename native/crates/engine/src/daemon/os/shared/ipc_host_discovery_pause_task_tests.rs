//! E49 (plan 2026-11-08), all surfaces: a renewal in its pause is in the
//! worker's snapshot, which CLI (`se share discoverable list`, `se share
//! status`), MCP (through `se`), Desktop and Android read, and the person's
//! stop of the replaced offer in the pause (`se share discoverable stop`,
//! „Nicht erneuern“) cancels it, so no offer with a new PIN follows.
use super::state_snapshot;
use crate::daemon::ipc_host::ShareHost;
use crate::daemon::ipc_protocol::{ShareCommandReply, ShareWorkerSnapshot};
use crate::share::{
    DiscoveryCommand, DiscoveryEvent, DiscoveryOfferStopReason, DiscoveryPublishTarget, ShareCmd,
};

const REPLACED: &str = "pin-neu-pause-angebot";
/// The offer is published and ends at this instant.
const START: i64 = 1_000;
/// E49: the pause before the renewal (`PIN_RENEWAL_PAUSE_SECS`).
const PIN_RENEWAL_PAUSE_SECS: i64 = 30;

#[test]
fn pin_neu_task_stop_in_the_pause_cancels_the_renewal() {
    let host = ShareHost::new("8".repeat(32));
    {
        let mut state = host.state.lock().expect("worker state");
        let target = DiscoveryPublishTarget::Direct;
        for event in [
            DiscoveryEvent::OfferPrepared {
                offer_id: REPLACED.into(),
                target: target.clone(),
                display_alias: "Laptop".into(),
                discoverable_until: START + 1_800,
            },
            DiscoveryEvent::OfferPublished {
                offer_id: REPLACED.into(),
                target,
                display_alias: "Laptop".into(),
                discoverable_until: START + 1_800,
            },
            DiscoveryEvent::OfferStopped {
                offer_id: REPLACED.into(),
                reason: DiscoveryOfferStopReason::TooManyFailedAttempts,
            },
        ] {
            state.discovery_offers.observe_at(&event, START);
        }
        let snapshot = state_snapshot(&mut state, false, START + 10);
        let [waiting] = snapshot.discovery_renewals.as_slice() else {
            panic!(
                "the pause is not in the snapshot: {:?}",
                snapshot.discovery_renewals
            );
        };
        assert_eq!(waiting.replaces, REPLACED);
        assert_eq!(waiting.target, DiscoveryPublishTarget::Direct);
        assert_eq!(waiting.display_alias, "Laptop");
        assert_eq!(waiting.due, START + PIN_RENEWAL_PAUSE_SECS);
        // The surfaces read it from the wire; a worker predating it sends none.
        let wire = serde_json::to_value(&snapshot).expect("snapshot as JSON");
        assert_eq!(wire["discovery_renewals"][0]["replaces"], REPLACED);
        let mut older = wire.clone();
        older
            .as_object_mut()
            .expect("snapshot object")
            .remove("discovery_renewals");
        let older: ShareWorkerSnapshot = serde_json::from_value(older).expect("older snapshot");
        assert!(older.discovery_renewals.is_empty());
    }

    // The person stops the replaced offer during the pause.
    let stop = ShareCmd::Discovery(DiscoveryCommand::StopPublishing {
        offer_id: REPLACED.into(),
    });
    assert!(matches!(
        host.send_command(stop),
        Ok(ShareCommandReply::Applied)
    ));
    let mut state = host.state.lock().expect("worker state");
    assert!(state_snapshot(&mut state, false, START + 20)
        .discovery_renewals
        .is_empty());
    assert!(state
        .discovery_offers
        .claim_due_renewals(START + PIN_RENEWAL_PAUSE_SECS + 1)
        .is_empty());
}
