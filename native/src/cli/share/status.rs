use clap::Args;

use super::{discoverable_output, grants, lifecycle_output};

const RECENT_TEXT_EVENTS: usize = 20;

#[derive(Args, Default)]
pub(super) struct StatusArgs {
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

pub(super) fn run(args: StatusArgs) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    // A copy of the recent events; the desktop app still receives them.
    let (snapshot, worker_error) = match crate::daemon::share_worker_snapshot() {
        Ok(snapshot) => (snapshot, None),
        Err(error) => (crate::daemon::ShareWorkerSnapshot::default(), Some(error)),
    };
    if args.json {
        print_json(&snapshot, worker_error.as_deref(), &profiles)
    } else {
        print_text(&snapshot, worker_error.as_deref(), &profiles);
        Ok(())
    }
}

fn print_json(
    snapshot: &crate::daemon::ShareWorkerSnapshot,
    worker_error: Option<&str>,
    profiles: &crate::share::ShareProfiles,
) -> Result<(), String> {
    let contacts = profiles.direct_contacts.iter().map(|contact| {
        serde_json::json!({
            "id": contact.id,
            "name": contact.display_name,
            "status": contact.status.label(),
            "access": contact.access_state.label(),
            "connectivity": {
                "state": share_status_code(&contact.status),
                "label": contact.status.label(),
                "last_seen": contact.last_seen,
                "last_error": contact.last_error,
            },
            "authorization": {
                "state": access_state_code(&contact.access_state),
                "active": contact.access_state == crate::share::DirectAccessState::Accepted,
                "accepted_at": contact.accepted_at,
            },
            "fingerprint": contact.expected_fingerprint,
        })
    });
    let rooms = profiles.rooms.iter().map(|room| {
        serde_json::json!({
            "id": room.id,
            "room_id": room.room_id,
            "name": room.name,
            "status": room.status.label(),
            "connectivity": {
                "state": share_status_code(&room.status),
                "label": room.status.label(),
            },
            "members": room.members.len(),
        })
    });
    let legacy_requests = profiles
        .legacy_direct_requests
        .iter()
        .map(|entry| super::requests_legacy::value(entry, profiles))
        .collect::<Vec<_>>();
    let requests = profiles
        .direct_requests
        .iter()
        .map(|entry| lifecycle_output::request_value(entry, profiles))
        .collect::<Vec<_>>();
    let value = serde_json::json!({
        "worker": {
            "reachable": worker_error.is_none(),
            "running": worker_error.is_none().then_some(snapshot.running),
            "connected": worker_error.is_none().then_some(snapshot.connected),
            "last_error": worker_error.or(snapshot.last_error.as_deref()),
            "relay_url": snapshot.relay_url,
            "candidates": snapshot.candidates,
        },
        // Keep the original top-level fields for existing scripts.
        "running": worker_error.is_none().then_some(snapshot.running),
        "connected": worker_error.is_none().then_some(snapshot.connected),
        "last_error": worker_error.or(snapshot.last_error.as_deref()),
        "relay_url": snapshot.relay_url,
        "candidates": snapshot.candidates,
        "contacts": contacts.collect::<Vec<_>>(),
        "rooms": rooms.collect::<Vec<_>>(),
        "requests": requests,
        "legacy_requests": legacy_requests,
        "grants": grants::values(profiles),
        "pending_requests": profiles.legacy_direct_requests.iter()
            .filter(|entry| entry.is_pending(crate::share::core_now_secs()))
            .map(|entry| super::requests_legacy::value(entry, profiles))
            .collect::<Vec<_>>(),
        "events": snapshot.events.iter().filter_map(public_event).collect::<Vec<_>>(),
        "discoverable": discoverable_values(snapshot, profiles),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn print_text(
    snapshot: &crate::daemon::ShareWorkerSnapshot,
    worker_error: Option<&str>,
    profiles: &crate::share::ShareProfiles,
) {
    println!("worker_reachable\t{}", worker_error.is_none());
    println!(
        "running\t{}",
        worker_error.map_or_else(|| snapshot.running.to_string(), |_| "unknown".into())
    );
    println!(
        "connected\t{}",
        worker_error.map_or_else(|| snapshot.connected.to_string(), |_| "unknown".into())
    );
    println!(
        "last_error\t{}",
        worker_error
            .or(snapshot.last_error.as_deref())
            .unwrap_or("-")
    );
    println!("relay_url\t{}", snapshot.relay_url);
    for contact in &profiles.direct_contacts {
        println!(
            "peer\t{}\t{}\t{}\t{}\tconnectivity={}\tauthorization={}\tlast_seen={}\tlast_error={}",
            contact.id,
            clean(&contact.display_name),
            clean(&contact.status.label()),
            clean(contact.access_state.label()),
            share_status_code(&contact.status),
            if contact.access_state == crate::share::DirectAccessState::Accepted {
                "active"
            } else {
                "inactive"
            },
            option_i64(contact.last_seen),
            contact
                .last_error
                .as_deref()
                .map(clean)
                .unwrap_or_else(|| "-".to_string()),
        );
    }
    for room in &profiles.rooms {
        println!(
            "room\t{}\t{}\t{}\t{}\tconnectivity={}",
            room.id,
            clean(&room.name),
            clean(&room.status.label()),
            room.members.len(),
            share_status_code(&room.status),
        );
    }
    for entry in &profiles.direct_requests {
        for line in lifecycle_output::request_text(entry, profiles) {
            println!("{line}");
        }
    }
    for entry in &profiles.legacy_direct_requests {
        println!("{}", super::requests_legacy::text(entry, profiles));
    }
    for line in grants::text(profiles) {
        println!("{line}");
    }
    // The worker keeps its recent events for the desktop app; show the newest.
    let events: Vec<String> = snapshot.events.iter().filter_map(public_event).collect();
    let newest = events.len().saturating_sub(RECENT_TEXT_EVENTS);
    for event in &events[newest..] {
        println!("event\t{event}");
    }
    let now = crate::share::core_now_secs();
    for offer in &snapshot.discovery_offers {
        println!("{}", discoverable_output::offer_text(offer, profiles, now));
    }
}

fn discoverable_values(
    snapshot: &crate::daemon::ShareWorkerSnapshot,
    profiles: &crate::share::ShareProfiles,
) -> Vec<serde_json::Value> {
    let now = crate::share::core_now_secs();
    snapshot
        .discovery_offers
        .iter()
        .map(|offer| discoverable_output::offer_value(offer, profiles, now))
        .collect()
}

fn public_event(event: &crate::share::ShareEvent) -> Option<String> {
    match event {
        crate::share::ShareEvent::Status(message) => Some(format!("status: {message}")),
        crate::share::ShareEvent::Error(message) => Some(format!("error: {message}")),
        crate::share::ShareEvent::ServerConnected => Some("server connected".to_string()),
        crate::share::ShareEvent::ServerDisconnected(message) => {
            Some(format!("server disconnected: {message}"))
        }
        crate::share::ShareEvent::DirectSignal(_) => None,
        _ => None,
    }
}

fn share_status_code(status: &crate::share::ShareStatus) -> &'static str {
    match status {
        crate::share::ShareStatus::Offline => "offline",
        crate::share::ShareStatus::Waiting => "waiting",
        crate::share::ShareStatus::WaitingForAccess => "waiting_for_access",
        crate::share::ShareStatus::Available => "available",
        crate::share::ShareStatus::Connecting => "connecting",
        crate::share::ShareStatus::Connected => "connected",
        crate::share::ShareStatus::ConnectedDirect => "connected_direct",
        crate::share::ShareStatus::ConnectedRelay => "connected_relay",
        crate::share::ShareStatus::Failed(_) => "failed",
        crate::share::ShareStatus::IdentityConflict => "identity_conflict",
    }
}

fn access_state_code(state: &crate::share::DirectAccessState) -> &'static str {
    match state {
        crate::share::DirectAccessState::Pending => "pending",
        crate::share::DirectAccessState::Accepted => "accepted",
        crate::share::DirectAccessState::Ignored => "ignored",
        crate::share::DirectAccessState::IdentityConflict => "identity_conflict",
    }
}

fn option_i64(value: Option<i64>) -> String {
    value.map_or_else(|| "-".to_string(), |value| value.to_string())
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}
