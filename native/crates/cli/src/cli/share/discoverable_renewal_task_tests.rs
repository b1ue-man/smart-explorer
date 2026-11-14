//! E49 on the CLI (and the MCP tool `se`, which runs it): a renewed offer
//! names the hint and its new PIN, a renewal in its pause is listed with the
//! seconds until the offer with a new PIN goes out, and `se share
//! discoverable stop` cancels it without a selection, with `--all`, and when
//! the selection names the replaced offer.
use super::super::discoverable_output::{offer_value, renewal_lines, waiting_text, waiting_value};
use super::renewal_stops;
use crate::share::{DiscoveryPublishTarget, OwnDiscoveryOffer, ShareProfiles, WaitingRenewal};

fn waiting(replaces: &str, due: i64) -> WaitingRenewal {
    WaitingRenewal {
        replaces: replaces.into(),
        target: DiscoveryPublishTarget::Direct,
        display_alias: "Laptop".into(),
        due,
    }
}

#[test]
fn pin_neu_task_cli_lists_and_stops_the_waiting_renewal() {
    let renewals = [waiting("ersetzt-a", 1_030), waiting("ersetzt-b", 1_040)];
    let profiles = ShareProfiles::default();

    assert_eq!(
        waiting_text(&renewals[0], &profiles, 1_010),
        "renewal\tersetzt-a\ttarget=direct\tname=Laptop\tdue_in=20\tstop=se share discoverable stop ersetzt-a"
    );
    let value = waiting_value(&renewals[0], &profiles, 1_010);
    assert_eq!(value["replaces"], "ersetzt-a");
    assert_eq!(value["due"], 1_030);
    assert_eq!(value["due_in_secs"], 20);
    assert_eq!(value["target"]["kind"], "direct");
    // After its due time the line counts no negative seconds.
    assert_eq!(
        waiting_value(&renewals[0], &profiles, 1_100)["due_in_secs"],
        0
    );

    let every = vec!["ersetzt-a".to_string(), "ersetzt-b".to_string()];
    assert_eq!(renewal_stops(&renewals, None, false), every);
    assert_eq!(renewal_stops(&renewals, None, true), every);
    assert_eq!(
        renewal_stops(&renewals, Some("ersetzt-b"), false),
        ["ersetzt-b"]
    );
    // A selection of a running offer cancels no renewal.
    assert!(renewal_stops(&renewals, Some("laufend"), false).is_empty());
    assert!(renewal_stops(&[], None, true).is_empty());
}

#[test]
fn pin_neu_task_discoverable_output_names_hint_and_new_pin() {
    let profiles = ShareProfiles::default();
    let mut offer = OwnDiscoveryOffer {
        offer_id: "o3".into(),
        target: DiscoveryPublishTarget::Direct,
        display_alias: "Laptop".into(),
        discoverable_until: 5_000,
        published: true,
        rights: None,
        renewed_after_failed_attempts: false,
        pin: None,
    };
    // An ordinary offer carries no hint and no PIN.
    assert!(renewal_lines(&offer).is_empty());
    let plain = offer_value(&offer, &profiles, 1_000);
    assert_eq!(plain["renewed_after_failed_attempts"], false);
    assert!(plain["hint"].is_null() && plain["pin"].is_null());

    // A renewed offer: text lines `hint` and `pin`, and the JSON keys.
    offer.renewed_after_failed_attempts = true;
    offer.pin = Some(crate::share::DiscoveryPin::new("482915".into()));
    assert_eq!(
        renewal_lines(&offer),
        [
            "hint\t5 falsche PIN-Versuche – vermutlich Rateversuche; neue PIN",
            "pin\t482915"
        ]
    );
    let value = offer_value(&offer, &profiles, 1_000);
    assert_eq!(value["renewed_after_failed_attempts"], true);
    assert_eq!(
        value["hint"],
        "5 falsche PIN-Versuche – vermutlich Rateversuche; neue PIN"
    );
    assert_eq!(value["pin"], "482915");
    assert!(!format!("{offer:?}").contains("482915"));
}
