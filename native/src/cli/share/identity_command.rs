use clap::Args;

const REPAIR_COMMAND: &str = "se share identity --repair";

#[derive(Args)]
pub(super) struct IdentityArgs {
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
    #[arg(
        long,
        help = "Repair identity secrets that are missing from secure storage"
    )]
    repair: bool,
}

pub(super) fn run(args: IdentityArgs) -> Result<(), String> {
    let identity = if args.repair {
        repair_identity()?
    } else {
        load_with_repair_hint()?
    };
    let direct_code = identity.direct_code();
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "device_id": identity.device_id,
                "device_name": identity.device_name,
                "fingerprint": identity.fingerprint,
                "node_id": identity.node_id,
                "direct_code": direct_code,
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!("device_id\t{}", identity.device_id);
        println!("device_name\t{}", identity.device_name);
        println!("fingerprint\t{}", identity.fingerprint);
        println!("node_id\t{}", identity.node_id);
        println!("direct_code\t{direct_code}");
    }
    Ok(())
}

pub(super) fn load_with_repair_hint() -> Result<crate::share::ShareIdentity, String> {
    crate::share::ShareIdentity::load_or_create(super::default_device_name()).map_err(|error| {
        if error.contains("fehlt im sicheren Speicher") {
            format!("{error}; repair with `{REPAIR_COMMAND}`")
        } else {
            error
        }
    })
}

fn repair_identity() -> Result<crate::share::ShareIdentity, String> {
    // Refuse healthy/corrupt identities before touching a live worker. The
    // mutating repair rechecks under its own transaction lock after the stop.
    crate::share::ShareIdentity::repair_action_needed(super::default_device_name())?;
    let worker = stop_share_worker_for_repair()?;
    let repaired = crate::share::ShareIdentity::repair_missing(super::default_device_name());
    if let Ok(outcome) = &repaired {
        match &outcome.action {
            crate::share::IdentityRepairAction::IdentityReplaced => eprintln!(
                "warning: The Share identity was replaced; old invites and trust relationships require re-pairing."
            ),
            crate::share::IdentityRepairAction::DirectCodeRotated => eprintln!(
                "warning: The direct code was rotated; the old invite is invalid."
            ),
        }
        if let Some(warning) = &outcome.cleanup_warning {
            eprintln!("warning: {warning}");
        }
    }
    let restored = restore_share_worker_after_repair(worker);
    match (repaired, restored) {
        (Ok(outcome), Ok(())) => Ok(outcome.identity),
        (Ok(_), Err(error)) => Err(format!("identity repaired, but {error}")),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(refresh)) => Err(format!("{error}; worker restore failed: {refresh}")),
    }
}

#[derive(Clone, Copy, Default)]
struct WorkerRepairState {
    daemon_running: bool,
    share_running: bool,
}

fn stop_share_worker_for_repair() -> Result<WorkerRepairState, String> {
    if !crate::daemon::is_running() {
        return Ok(WorkerRepairState::default());
    }
    let was_running = crate::daemon::drain_share_worker_events()
        .map_err(|error| format!("cannot inspect the Share worker before repair: {error}"))?
        .running;
    if !was_running {
        return Ok(WorkerRepairState {
            daemon_running: true,
            share_running: false,
        });
    }
    let stop_result = (|| {
        crate::daemon::send_share_command(crate::share::ShareCmd::Stop).map_err(|error| {
            format!(
                "identity repair refused because the Share worker could not be stopped: {error}"
            )
        })?;
        let stopped = crate::daemon::drain_share_worker_events().map_err(|error| {
            format!(
                "identity repair refused because the Share worker stop could not be verified: {error}"
            )
        })?;
        if stopped.running {
            return Err("identity repair refused because the Share worker is still running".into());
        }
        Ok(())
    })();
    if let Err(error) = stop_result {
        return match restore_share_worker_after_repair(WorkerRepairState {
            daemon_running: true,
            share_running: true,
        }) {
            Ok(()) => Err(error),
            Err(restore) => Err(format!("{error}; worker restore failed: {restore}")),
        };
    }
    Ok(WorkerRepairState {
        daemon_running: true,
        share_running: true,
    })
}

fn restore_share_worker_after_repair(worker: WorkerRepairState) -> Result<(), String> {
    if !worker.daemon_running {
        return Ok(());
    }
    match crate::daemon::refresh_share_worker_checked() {
        Ok(true) => Ok(()),
        Ok(false) if !worker.share_running => Ok(()),
        Ok(false) => Err("the Share worker did not restart".into()),
        Err(error) => Err(format!("the Share worker could not be refreshed: {error}")),
    }
}
