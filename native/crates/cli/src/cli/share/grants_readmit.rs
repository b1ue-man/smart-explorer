//! Deliberate reactivation of an inactive incoming Direct grant.
use clap::Args;

#[derive(Args)]
pub(super) struct AllowArgs {
    #[arg(
        allow_hyphen_values = true,
        help = "Device id/name/fingerprint or request selector; omit when exactly one inactive grant exists"
    )]
    selector: Option<String>,
    #[arg(long, help = "Optional extra fingerprint assertion")]
    fingerprint: Option<String>,
    #[arg(
        long,
        conflicts_with_all = ["selector", "fingerprint"],
        help = "Confirm every grant that waits for re-confirmation after a code rotation or identity repair (blocked grants stay blocked; Exec stays off)"
    )]
    all_reconfirm: bool,
}

pub(super) fn run(args: AllowArgs, json: bool) -> Result<(), String> {
    if args.all_reconfirm {
        return all_reconfirm(json);
    }
    let profiles = super::super::checked_profiles()?;
    let matches = profiles
        .direct_grants
        .iter()
        .filter(|grant| grant.state != crate::share::DirectGrantState::Accepted)
        .filter(|grant| {
            args.selector.as_deref().is_none_or(|selector| {
                super::grant_selector_matches(&profiles, grant, selector.trim())
            })
        })
        .collect::<Vec<_>>();
    let grant = match matches.as_slice() {
        [grant] => *grant,
        [] => return Err("inactive grant not found; run `se share grants`".into()),
        _ => {
            return Err(
                "multiple inactive grants; choose one selector from `se share grants`".into(),
            );
        }
    };
    super::super::request_selection::verify_optional_fingerprint(
        args.fingerprint.as_deref(),
        &grant.fingerprint,
        &grant.device_id,
    )?;
    let device_id = grant.device_id.clone();
    let change =
        crate::share::allow_direct_peer_again(Some(super::super::default_home()), &device_id)?;
    let (worker_state, worker_error) = super::worker_refresh();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "action": "allowed",
                "device_id": device_id,
                "persisted": true,
                "changed": change.changed,
                "exec_enabled": false,
                "grants": super::values(&change.profiles),
                "worker_refresh": {"state": worker_state, "error": worker_error},
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "action\tallowed\tdevice_id={}\tpersisted=true\texec=disabled\tworker_refresh={}",
            super::clean(&device_id),
            worker_state,
        );
        if let Some(error) = worker_error {
            println!("worker_error\t{}", super::clean(&error));
        }
    }
    Ok(())
}

/// R9 (plan 2026-11-06): „Alle neu bestätigen“.
fn all_reconfirm(json: bool) -> Result<(), String> {
    let before = super::super::checked_profiles()?;
    let devices: Vec<String> = before
        .direct_grants
        .iter()
        .filter(|grant| grant.state == crate::share::DirectGrantState::Reconfirm)
        .map(|grant| grant.device_id.clone())
        .collect();
    let change = crate::share::lifecycle_view::access_gaps::reconfirm_all_direct_grants(Some(
        super::super::default_home(),
    ))?;
    let (worker_state, worker_error) = super::worker_refresh();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "action": "reconfirmed",
                "device_ids": devices,
                "persisted": true,
                "changed": change.changed,
                "exec_enabled": false,
                "grants": super::values(&change.profiles),
                "worker_refresh": {"state": worker_state, "error": worker_error},
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "action\treconfirmed\tdevices={}\tpersisted=true\texec=disabled\tworker_refresh={}",
            devices.len(),
            worker_state,
        );
        for device in &devices {
            println!("reconfirmed\t{}", super::clean(device));
        }
        if let Some(error) = worker_error {
            println!("worker_error\t{}", super::clean(&error));
        }
    }
    Ok(())
}
