use clap::Args;

const REPAIR_COMMAND: &str = "se share identity --repair";

#[derive(Args)]
#[command(group(
    clap::ArgGroup::new("identity_change")
        .args(["repair", "set_name", "direct_online", "rotate_code"])
        .multiple(false)
))]
pub(super) struct IdentityArgs {
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
    #[arg(
        long,
        help = "Repair identity secrets that are missing from secure storage"
    )]
    repair: bool,
    #[arg(
        long,
        value_name = "NAME",
        help = "Change this device's Share name (shown to paired devices and rooms)"
    )]
    set_name: Option<String>,
    #[arg(
        long,
        value_name = "true|false",
        help = "Make the direct code reachable (true) or unreachable (false) for new contacts until the worker restarts; paired devices and rooms stay connected"
    )]
    direct_online: Option<bool>,
    #[arg(
        long,
        help = "Create a new direct code: the old invite stops working, existing direct authorizations pause until confirmed again and their Exec turns off"
    )]
    rotate_code: bool,
    #[arg(
        long,
        requires = "rotate_code",
        help = "Confirm --rotate-code without a prompt"
    )]
    yes: bool,
}

pub(super) fn run(args: IdentityArgs) -> Result<(), String> {
    if let Some(online) = args.direct_online {
        crate::daemon::send_share_command(crate::share::ShareCmd::SetDirectOnline { online })
            .map_err(|error| format!("direct code state not changed: {error}"))?;
        println!("direct_online\t{online}\tuntil the Share worker restarts");
        return Ok(());
    }
    let identity = if args.repair {
        repair_identity()?
    } else if let Some(name) = args.set_name {
        let mut identity = load_with_repair_hint()?;
        identity.set_device_name(name)?;
        eprintln!("device name changed{}", super::refresh_note());
        identity
    } else if args.rotate_code {
        rotate_direct_code(args.yes)?
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
    let cleanup_failed = repaired
        .as_ref()
        .ok()
        .and_then(|outcome| outcome.cleanup_warning.as_ref())
        .is_some();
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
    let restored = if cleanup_failed && worker.share_running {
        Err(
            "identity repaired, but authorization cleanup failed; Share worker remains stopped"
                .into(),
        )
    } else {
        restore_share_worker_after_repair(worker)
    };
    match (repaired, restored) {
        (Ok(outcome), Ok(())) => Ok(outcome.identity),
        (Ok(_), Err(error)) => Err(format!("identity repaired, but {error}")),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(refresh)) => Err(format!("{error}; worker restore failed: {refresh}")),
    }
}

/// „Code neu generieren“ of the desktop page „Dieses Gerät“: the worker is
/// stopped, the code renewed, pending direct authorizations are paused while
/// it stays stopped, then the worker returns.
fn rotate_direct_code(confirmed: bool) -> Result<crate::share::ShareIdentity, String> {
    if !confirmed {
        confirm_rotation()?;
    }
    let mut identity = load_with_repair_hint()?;
    let worker = stop_share_worker_for_repair()?;
    let rotation = match identity.regenerate_direct_code() {
        Ok(rotation) => rotation,
        Err(error) => {
            return match restore_share_worker_after_repair(worker) {
                Ok(()) => Err(format!("direct code not renewed: {error}")),
                Err(restore) => Err(format!(
                    "direct code not renewed: {error}; worker restore failed: {restore}"
                )),
            };
        }
    };
    if let Err(error) = identity.complete_pending_cleanup(Some(super::default_home())) {
        return Err(format!(
            "direct code renewed, but the Share worker stays stopped for safety: direct authorizations could not be paused: {error}"
        ));
    }
    restore_share_worker_after_repair(worker)
        .map_err(|error| format!("direct code renewed, but {error}"))?;
    eprintln!("warning: The direct code was renewed; the old invite is invalid.");
    if let Some(warning) = rotation.cleanup_warning {
        eprintln!("warning: {warning}");
    }
    Ok(identity)
}

fn confirm_rotation() -> Result<(), String> {
    use std::io::Write;
    eprintln!(
        "A new direct code invalidates the old one. Direct authorizations pause until they are confirmed again; their Exec turns off."
    );
    eprint!("Type YES to renew the direct code: ");
    std::io::stderr()
        .flush()
        .map_err(|error| error.to_string())?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|error| error.to_string())?;
    (answer.trim() == "YES")
        .then_some(())
        .ok_or_else(|| "direct code was not renewed".into())
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
    let was_running = crate::daemon::share_worker_snapshot()
        .map_err(|error| {
            format!("cannot inspect the Share worker before the identity change: {error}")
        })?
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
                "identity change refused because the Share worker could not be stopped: {error}"
            )
        })?;
        let stopped = crate::daemon::share_worker_snapshot().map_err(|error| {
            format!(
                "identity change refused because the Share worker stop could not be verified: {error}"
            )
        })?;
        if stopped.running {
            return Err("identity change refused because the Share worker is still running".into());
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
