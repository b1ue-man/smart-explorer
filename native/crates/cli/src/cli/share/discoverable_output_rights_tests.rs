//! Plan 2026-10-31 F5: `se share discoverable` and `se share status` name
//! the rights a running PIN offer gives (text and JSON share one shape).
use super::{offer_text, offer_value};
use crate::share::connection_rights::RightsPreset;
use crate::share::{DiscoveryPublishTarget, OwnDiscoveryOffer, ShareProfiles};

fn offer(rights: Option<crate::share::connection_rights::ConnectionRights>) -> OwnDiscoveryOffer {
    OwnDiscoveryOffer {
        offer_id: "o1".into(),
        target: DiscoveryPublishTarget::Direct,
        display_alias: "Laptop".into(),
        discoverable_until: 1_299,
        published: true,
        rights,
        renewed_after_failed_attempts: false,
        pin: None,
    }
}

#[test]
fn abgleich_task_discoverable_text_names_the_given_preset() {
    let profiles = ShareProfiles::default();
    let text = offer_text(
        &offer(Some(RightsPreset::ReadOnly.rights())),
        &profiles,
        1_000,
    );
    assert!(
        text.ends_with("\tremaining=4m59s\trights=Nur lesen"),
        "{text}"
    );
    let text = offer_text(&offer(Some(RightsPreset::All.rights())), &profiles, 1_000);
    assert!(text.ends_with("\trights=Alles"), "{text}");
}

#[test]
fn abgleich_task_discoverable_json_carries_rights_and_summary() {
    let profiles = ShareProfiles::default();
    let value = offer_value(
        &offer(Some(RightsPreset::FilesExec.rights())),
        &profiles,
        1_000,
    );
    assert_eq!(value["rights_summary"], "Dateien und Codeausführung");
    assert_eq!(value["rights"]["preset"], "files-exec");
    assert_eq!(value["rights"]["write"], true);
    assert_eq!(value["rights"]["commands"], "allow");
    assert_eq!(value["rights"]["terminal"], "allow");
    assert_eq!(value["rights"]["screen_view"], "off");

    // No rights: the device connects without any and waits for the menu.
    let waiting = offer_value(&offer(None), &profiles, 1_000);
    assert!(waiting["rights"].is_null());
    assert_eq!(waiting["rights_summary"], "waits for confirmation");
}
