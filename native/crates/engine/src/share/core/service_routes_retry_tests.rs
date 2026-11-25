//! Status fields of direct retries in relay fallback (2026-11-25).
use super::*;
use crate::share::route_state::{RouteClass, RouteStatus};

fn fallback_facts(retry: Option<DirectRetryFacts>) -> RouteDiagnostics {
    let mut status = RouteStatus::initial(140);
    status.phase = RoutePhase::RelayFallback;
    status.class = Some(RouteClass::Relay);
    status.application_allowed = true;
    status.reason = "Relay als Rückfall; neuer Direktversuch in 20 s (Versuch 3); Absolute \
                     Direktfrist abgelaufen; im aktuellen Versuch kein validierter Direktpfad"
        .into();
    RouteDiagnostics {
        connection_id: 7,
        protocol: "smart-explorer/share-fs/3".into(),
        status,
        budget: RouteBudget::default(),
        measurements: Vec::new(),
        retry,
    }
}

#[test]
fn route_retry_task_status_shows_next_attempt_and_count() {
    let facts = fallback_facts(Some(DirectRetryFacts {
        attempts: 2,
        next_attempt_in_ms: Some(12_500),
        next_attempt_unix_ms: Some(1_764_000_012_500),
        last_cause: Some(RetryCause::PeerAddresses),
        last_nat_round: Some(true),
    }));
    let view = ConnectionRouteDiagnostics::from_facts(&facts);
    let json = serde_json::to_value(&view).expect("json");
    assert_eq!(json["state"], "relay_fallback");
    assert_eq!(json["pathClass"], "relay");
    assert!(json["reason"]
        .as_str()
        .is_some_and(|reason| reason
            .starts_with("Relay als Rückfall; neuer Direktversuch in 20 s (Versuch 3)")));
    let retry = &json["directRetry"];
    assert_eq!(retry["directAttempts"], 2);
    assert_eq!(retry["nextDirectAttemptInMs"], 12_500);
    assert_eq!(retry["nextDirectAttemptUnixMs"], 1_764_000_012_500u64);
    assert_eq!(retry["lastDirectAttemptCause"], "peer_addresses");
    assert_eq!(retry["lastDirectAttemptNatRound"], true);
    let line = "Direktversuche seit dem Relay-Rückfall: 2; nächster in 13 s; letzter Anlass: \
                Gegenstelle hat neue Adressen veröffentlicht; Lochungsrunde gestartet";
    assert!(
        view.detail_lines().iter().any(|detail| detail == line),
        "{:?}",
        view.detail_lines()
    );
    // Without retries (direct, server side) the field is absent.
    let plain = serde_json::to_value(ConnectionRouteDiagnostics::from_facts(&fallback_facts(
        None,
    )))
    .expect("json");
    assert!(plain.get("directRetry").is_none());
    // Facts of an older writer without the field still parse.
    let mut old = serde_json::to_value(&facts).expect("json");
    old.as_object_mut().expect("object").remove("retry");
    let parsed: RouteDiagnostics = serde_json::from_value(old).expect("older facts");
    assert!(parsed.retry.is_none());
}
