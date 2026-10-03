use clap::{Args, Subcommand};
use serde_json::json;

use super::{export_config, exports_policy, ExportListArgs, ExportScope};
use crate::share::{ExportAccess, ShareProfiles};

#[derive(Args)]
pub(super) struct ConnectionsArgs {
    #[command(subcommand)]
    command: ConnectionCommand,
}

#[derive(Subcommand)]
enum ConnectionCommand {
    List(ExportListArgs),
    #[command(
        about = "Share exactly one saved account (new entries read-only), change its rights, or withdraw it"
    )]
    Set(SetArgs),
}

#[derive(Args)]
struct SetArgs {
    #[arg(help = "Exact SavedConnection account shown by connections list")]
    account: String,
    #[arg(long, conflicts_with_all = ["read_only", "remove"], help = "Allow writing through your saved credentials")]
    write: bool,
    #[arg(long, conflicts_with = "remove")]
    read_only: bool,
    #[arg(long, help = "Withdraw this account's export")]
    remove: bool,
    #[command(flatten)]
    scope: ExportScope,
}

pub(super) fn run(args: ConnectionsArgs) -> Result<(), String> {
    match args.command {
        ConnectionCommand::List(args) => list(args),
        ConnectionCommand::Set(args) => set(args),
    }
}

fn list(args: ExportListArgs) -> Result<(), String> {
    let profiles = super::super::checked_profiles()?;
    let config = export_config(&profiles, args.scope.room.as_deref())?;
    let saved = crate::creds::load_connections_checked()?;
    let connections = saved.iter().map(|connection| {
        let account = connection.account();
        json!({"account": account, "label": connection.display(), "access": config.connection_access(&account)})
    }).collect::<Vec<_>>();
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "warning": "Peers use your saved credentials for each selected connection",
                "connections": connections,
                "shared_connections": config.shared_connections,
            }))
            .map_err(|error| error.to_string())?
        );
    } else {
        println!("warning\tPeers use your saved credentials for each selected connection");
        for connection in saved {
            let account = connection.account();
            let access = config
                .connection_access(&account)
                .map(exports_policy::access_name)
                .unwrap_or("not_shared");
            println!(
                "connection\t{account}\tlabel={}\taccess={access}",
                connection.display()
            );
        }
        for connection in &config.shared_connections {
            println!(
                "shared_connection\t{}\taccess={}",
                connection.account,
                exports_policy::access_name(connection.access)
            );
        }
    }
    Ok(())
}

fn set(args: SetArgs) -> Result<(), String> {
    if !args.remove
        && !crate::creds::load_connections_checked()?
            .iter()
            .any(|saved| saved.account() == args.account)
    {
        return Err("saved account not found; use the exact account from connections list".into());
    }
    let before = super::super::checked_profiles()?;
    let scope = exports_policy::scope_id(&before, args.scope.room.as_deref())?;
    let requested = if args.write {
        Some(ExportAccess::ReadWrite)
    } else if args.read_only {
        Some(ExportAccess::ReadOnly)
    } else {
        None
    };
    let mut committed_access = None;
    ShareProfiles::mutate_persisted(Some(super::super::default_home()), |profiles| {
        let config = profiles.export_config_mut(&scope)?;
        let access = if args.remove {
            None
        } else {
            requested
                .or(config.connection_access(&args.account))
                .or(Some(ExportAccess::ReadOnly))
        };
        config.set_connection_access(&args.account, access)?;
        committed_access = access;
        Ok(())
    })?;
    println!(
        "Updated connection {}: {}{}",
        args.account,
        committed_access
            .map(exports_policy::access_name)
            .unwrap_or("not_shared"),
        super::super::refresh_note()
    );
    Ok(())
}
