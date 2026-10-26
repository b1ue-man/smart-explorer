//! Fixed pairing failures and public metadata only; PIN text never enters output.
use serde_json::{json, Value};

use crate::daemon::{DiscoveryPairingError, DiscoveryPairingReply, DiscoveryPairingStatus};
use super::discoverable_output::local_time;

pub(super) fn list(offers: &[crate::share::DiscoveryAdvertisement], json_output: bool) {
    let now = crate::share::core_now_secs();
    if json_output {
        println!("{}", json!({"state": "list", "offers": offers.iter().map(|offer| json!({
            "discovery_id": offer.discovery_id,
            "offer_id": offer.offer_id,
            "display_name": offer.display_alias,
            "kind": offer.kind,
            "expires_at": offer.expires_at,
            "compatible": offer.is_compatible(),
            "status": availability(offer, now)
        })).collect::<Vec<_>>() }));
    } else if offers.is_empty() {
        println!("No discoverable offers. The peer must publish an offer first.");
    } else {
        println!("OFFER\tNAME\tKIND\tEXPIRES_AT\tSTATUS");
        for offer in offers {
            println!("{}\t{}\t{}\t{}\t{}", clean(&offer.discovery_id), clean(&offer.display_alias),
                offer.kind.wire_tag(), local_time(offer.expires_at),
                availability(offer, now));
        }
    }
}

pub(super) fn finished(reply: &DiscoveryPairingReply, json_output: bool) -> i32 {
    let DiscoveryPairingReply::Finished { discovery_id, exchange_id, status, outcome, error_code } = reply else {
        return failure(None, None, DiscoveryPairingError::DaemonError, json_output);
    };
    let (label, exit_code) = match status {
        DiscoveryPairingStatus::Completed => ("completed", 0),
        DiscoveryPairingStatus::Unconfirmed => ("unconfirmed", 2),
        DiscoveryPairingStatus::Cancelled => ("cancelled", 1),
        DiscoveryPairingStatus::Failed => ("failed", 1),
    };
    if json_output {
        println!("{}", serde_json::to_value(reply).unwrap_or_else(|_| json!({"state":"error","error_code":"daemon_error"})));
    } else {
        println!("offer\t{}", clean(discovery_id));
        println!("exchange\t{}", clean(exchange_id));
        println!("status\t{label}");
        if let Some(outcome) = outcome {
            let value = serde_json::to_value(outcome).unwrap_or(Value::Null);
            println!("relation\t{value}");
        }
        if *status == DiscoveryPairingStatus::Unconfirmed {
            println!("The relation was installed; peer confirmation is missing. Inspect the contact or room before retrying.");
        }
        if let Some(error) = error_code { println!("error\t{}\t{}", error.code(), error.message()); }
    }
    exit_code
}

pub(super) fn failure(
    discovery_id: Option<&str>,
    exchange_id: Option<&str>,
    error: DiscoveryPairingError,
    json_output: bool,
) -> i32 {
    failure_text(discovery_id, exchange_id, error.code(), error.message(), json_output)
}

pub(super) fn failure_text(
    discovery_id: Option<&str>,
    exchange_id: Option<&str>,
    code: &str,
    message: &str,
    json_output: bool,
) -> i32 {
    if json_output {
        println!("{}", json!({"state":"failed", "discovery_id":discovery_id,
            "exchange_id":exchange_id, "error_code":code, "message":message}));
    } else {
        eprintln!("{code}: {message}");
    }
    1
}

fn clean(value: &str) -> String {
    value.chars().map(|ch| if ch.is_control() { ' ' } else { ch }).collect()
}

fn availability(offer: &crate::share::DiscoveryAdvertisement, now: i64) -> &'static str {
    if offer.expires_at <= now {
        "expired"
    } else if offer.is_compatible() {
        "available"
    } else {
        "incompatible"
    }
}
