//! Text and JSON shapes of this device's discovery offers, shared by
//! `se share discoverable` and `se share status`.
use crate::share::{
    DiscoveryOfferStopReason, DiscoveryPublishTarget, OwnDiscoveryOffer, ShareProfiles,
};

pub(super) fn offer_value(
    offer: &OwnDiscoveryOffer,
    profiles: &ShareProfiles,
    now: i64,
) -> serde_json::Value {
    serde_json::json!({
        "offer_id": offer.offer_id,
        "state": state_code(offer),
        "target": target_value(&offer.target, profiles),
        "name": offer.display_alias,
        "discoverable_until": offer.discoverable_until,
        "discoverable_until_local": local_time(offer.discoverable_until),
        "remaining_secs": remaining_secs(offer, now),
    })
}

pub(super) fn offer_text(offer: &OwnDiscoveryOffer, profiles: &ShareProfiles, now: i64) -> String {
    format!(
        "discoverable\t{}\tstate={}\ttarget={}\tname={}\tuntil={}\tremaining={}",
        offer.offer_id,
        state_code(offer),
        target_text(&offer.target, profiles),
        clean(&offer.display_alias),
        local_time(offer.discoverable_until),
        remaining(remaining_secs(offer, now)),
    )
}

pub(super) fn stop_reason_text(reason: DiscoveryOfferStopReason) -> &'static str {
    match reason {
        DiscoveryOfferStopReason::Requested => "it was stopped on request",
        DiscoveryOfferStopReason::Expired => "its time ran out",
        DiscoveryOfferStopReason::TargetUnavailable => "the target is no longer available",
        DiscoveryOfferStopReason::CapabilityUnavailable => {
            "the Share server does not support discovery"
        }
        DiscoveryOfferStopReason::TransportError => {
            "the Share server rejected it or the connection failed"
        }
        DiscoveryOfferStopReason::WorkerStopped => "the Share worker stopped or restarted",
    }
}

pub(super) fn target_text(target: &DiscoveryPublishTarget, profiles: &ShareProfiles) -> String {
    match target {
        DiscoveryPublishTarget::Direct => "direct".to_string(),
        DiscoveryPublishTarget::Room { room_profile_id } => {
            let name = room_name(profiles, room_profile_id);
            format!("room:{}", clean(name.unwrap_or(room_profile_id)))
        }
    }
}

fn target_value(target: &DiscoveryPublishTarget, profiles: &ShareProfiles) -> serde_json::Value {
    match target {
        DiscoveryPublishTarget::Direct => serde_json::json!({ "kind": "direct" }),
        DiscoveryPublishTarget::Room { room_profile_id } => serde_json::json!({
            "kind": "room",
            "room_profile_id": room_profile_id,
            "room_name": room_name(profiles, room_profile_id),
        }),
    }
}

fn room_name<'a>(profiles: &'a ShareProfiles, room_profile_id: &str) -> Option<&'a str> {
    profiles
        .rooms
        .iter()
        .find(|room| room.id == room_profile_id)
        .map(|room| room.name.as_str())
}

fn state_code(offer: &OwnDiscoveryOffer) -> &'static str {
    if offer.published {
        "published"
    } else {
        "prepared"
    }
}

fn remaining_secs(offer: &OwnDiscoveryOffer, now: i64) -> i64 {
    offer.discoverable_until.saturating_sub(now).max(0)
}

fn remaining(secs: i64) -> String {
    let secs = secs.max(0);
    let (hours, minutes, seconds) = (secs / 3600, secs % 3600 / 60, secs % 60);
    if hours > 0 {
        format!("{hours}h{minutes:02}m")
    } else {
        format!("{minutes}m{seconds:02}s")
    }
}

/// Local wall-clock time with its UTC offset, e.g. `2026-09-26 14:05:00 +02:00`.
pub(super) fn local_time(unix_secs: i64) -> String {
    match chrono::DateTime::<chrono::Utc>::from_timestamp(unix_secs, 0) {
        Some(time) => time
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S %:z")
            .to_string(),
        None => unix_secs.to_string(),
    }
}

pub(super) fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::{offer_text, offer_value, remaining, stop_reason_text};
    use crate::share::{
        DiscoveryOfferStopReason, DiscoveryPublishTarget, OwnDiscoveryOffer, RoomProfile,
        ShareProfiles,
    };

    #[test]
    fn cli_task_discoverable_offer_text_and_json() {
        let mut profiles = ShareProfiles::default();
        profiles.rooms.push(RoomProfile {
            id: "p1".into(),
            name: "Team".into(),
            room_id: "r1".into(),
            auto_join: true,
            last_seen: None,
            status: Default::default(),
            members: Vec::new(),
            exports: Default::default(),
        });
        let direct = OwnDiscoveryOffer {
            offer_id: "o1".into(),
            target: DiscoveryPublishTarget::Direct,
            display_alias: "Lap\ttop".into(),
            discoverable_until: 1_299,
            published: true,
        };
        let text = offer_text(&direct, &profiles, 1_000);
        assert!(text.starts_with(
            "discoverable\to1\tstate=published\ttarget=direct\tname=Lap top\tuntil="
        ));
        assert!(text.ends_with("\tremaining=4m59s"));

        let room = OwnDiscoveryOffer {
            offer_id: "o2".into(),
            target: DiscoveryPublishTarget::Room {
                room_profile_id: "p1".into(),
            },
            display_alias: "Team".into(),
            discoverable_until: 5_000,
            published: false,
        };
        assert!(offer_text(&room, &profiles, 1_000).contains("\ttarget=room:Team\t"));
        let value = offer_value(&room, &profiles, 1_000);
        assert_eq!(value["state"], "prepared");
        assert_eq!(value["target"]["kind"], "room");
        assert_eq!(value["target"]["room_name"], "Team");
        assert_eq!(value["discoverable_until"], 5_000);
        assert_eq!(value["remaining_secs"], 4_000);
        assert_eq!(offer_value(&direct, &profiles, 9_000)["remaining_secs"], 0);

        assert_eq!(remaining(3_725), "1h02m");
        assert_eq!(remaining(-5), "0m00s");
        assert_eq!(
            stop_reason_text(DiscoveryOfferStopReason::WorkerStopped),
            "the Share worker stopped or restarted"
        );
    }
}
