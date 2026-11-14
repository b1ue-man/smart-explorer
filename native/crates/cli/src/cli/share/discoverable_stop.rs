//! `se share discoverable stop`: ends the running offers the selection names
//! and, during their pause, the renewals of offers that ended after too many
//! failed attempts (E49), so no offer with a new PIN follows.
use super::super::discoverable_input::{renewal_stops, select_offers};
use super::super::discoverable_output::clean;
use super::StopArgs;
use crate::share::{DiscoveryCommand, ShareCmd};

pub(super) fn stop(args: StopArgs) -> Result<(), String> {
    let snapshot = crate::daemon::share_worker_snapshot()?;
    // E49: a renewal in its pause ends with a stop of the offer it replaces.
    let renewals = renewal_stops(
        &snapshot.discovery_renewals,
        args.offer.as_deref(),
        args.all,
    );
    let names_renewal = args.offer.is_some() && !renewals.is_empty();
    let selected = if names_renewal {
        Vec::new()
    } else {
        select_offers(&snapshot.discovery_offers, args.offer.as_deref(), args.all)?
    };
    let targets: Vec<String> = selected
        .iter()
        .map(|offer| offer.offer_id.clone())
        .chain(renewals)
        .collect();
    let mut stopped = Vec::new();
    let mut failed = Vec::new();
    for offer_id in &targets {
        let offer_id = offer_id.clone();
        let command = ShareCmd::Discovery(DiscoveryCommand::StopPublishing {
            offer_id: offer_id.clone(),
        });
        match crate::daemon::share_command(command) {
            Ok(_) => stopped.push(offer_id),
            Err(error) => failed.push((offer_id, error)),
        }
    }
    if args.json {
        let failed_values: Vec<_> = failed
            .iter()
            .map(|(offer_id, error)| serde_json::json!({ "offer_id": offer_id, "error": error }))
            .collect();
        let value = serde_json::json!({
            "action": "stop",
            "stopped": stopped,
            "failed": failed_values,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
        );
    } else {
        if targets.is_empty() {
            println!("stopped\tnone\tno offer is discoverable");
        }
        for offer_id in &stopped {
            println!("stopped\t{offer_id}");
        }
        for (offer_id, error) in &failed {
            println!("failed\t{offer_id}\t{}", clean(error));
        }
    }
    if failed.is_empty() {
        return Ok(());
    }
    let (failures, total) = (failed.len(), targets.len());
    Err(format!("{failures} of {total} offers could not be stopped"))
}
