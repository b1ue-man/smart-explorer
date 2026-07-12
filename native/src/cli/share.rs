use clap::{Args, Subcommand};
use std::io::Write;
use std::path::{Path, PathBuf};

mod exports;
#[path = "share/grants.rs"]
mod grants;
#[path = "share/identity_command.rs"]
mod identity_command;
#[path = "share/lifecycle_output.rs"]
mod lifecycle_output;
#[path = "share/request_selection.rs"]
mod request_selection;
#[path = "share/requests.rs"]
mod requests;
#[path = "share/status.rs"]
mod status;

const MAX_SERVER_BYTES: usize = 16 * 1024;

#[derive(Args)]
pub(super) struct ShareArgs {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Configure and start the headless Share worker")]
    Configure(ConfigureArgs),
    #[command(about = "Show this device's Share identity and direct invite code")]
    Identity(identity_command::IdentityArgs),
    #[command(about = "Show worker, connectivity, request, and authorization status")]
    Status(status::StatusArgs),
    #[command(about = "Inspect and decide durable direct access requests")]
    Request(requests::RequestArgs),
    #[command(about = "Inspect and revoke direct authorization grants")]
    Grants(grants::GrantsArgs),
    #[command(about = "Manage local folders exported to peers")]
    Export(exports::ExportArgs),
    #[command(about = "Create a Share room and print its invite code")]
    Room(RoomArgs),
    #[command(about = "Refresh or stop the headless Share worker")]
    Worker(WorkerArgs),
}

#[derive(Args)]
struct ConfigureArgs {
    #[arg(long, help = "Signaling endpoint (host, tcp://, ws://, or wss://)")]
    server: String,
    #[arg(long, help = "Persist a device name with the Share identity")]
    device_name: Option<String>,
}

#[derive(Args)]
struct RoomArgs {
    #[command(subcommand)]
    command: RoomCommand,
}

#[derive(Subcommand)]
enum RoomCommand {
    Create {
        #[arg(long, default_value = "", help = "Local room display name")]
        name: String,
    },
}

#[derive(Args)]
struct WorkerArgs {
    #[command(subcommand)]
    command: WorkerCommand,
}

#[derive(Subcommand)]
enum WorkerCommand {
    Refresh,
    Stop,
}

pub(super) fn run(args: ShareArgs) -> Result<i32, String> {
    match args.command {
        None => status::run(status::StatusArgs::default())?,
        Some(Command::Configure(args)) => configure(args)?,
        Some(Command::Identity(args)) => identity_command::run(args)?,
        Some(Command::Status(args)) => status::run(args)?,
        Some(Command::Request(args)) => requests::run(args)?,
        Some(Command::Grants(args)) => grants::run(args)?,
        Some(Command::Export(args)) => exports::run(args)?,
        Some(Command::Room(args)) => match args.command {
            RoomCommand::Create { name } => create_room(&name)?,
        },
        Some(Command::Worker(args)) => worker(args.command)?,
    }
    Ok(0)
}

fn configure(args: ConfigureArgs) -> Result<(), String> {
    let server = validate_server(&args.server)?;
    let mut identity = identity_command::load_with_repair_hint()?;
    if let Some(name) = args.device_name {
        identity.set_device_name(name)?;
    }
    let profiles = checked_profiles()?;
    if !profiles.auto_connect {
        crate::share::ShareProfiles::mutate_persisted(Some(default_home()), |candidate| {
            candidate.auto_connect = true;
            Ok(())
        })?;
    }
    write_atomic(
        &crate::support_dirs::app_data_file("share_server.txt"),
        &server,
    )?;
    refresh_required()?;
    println!("Share worker configured for {server}");
    Ok(())
}

fn create_room(name: &str) -> Result<(), String> {
    let code = crate::share::ShareProfiles::new_room_code()?;
    let (_profiles, id) = crate::share::ShareProfiles::add_room_from_code_persisted(
        Some(default_home()),
        &code,
        name,
    )?;
    println!("room_id\t{id}");
    println!("room_code\t{code}");
    println!("worker\t{}", refresh_note().trim_start_matches("; "));
    Ok(())
}

fn worker(command: WorkerCommand) -> Result<(), String> {
    match command {
        WorkerCommand::Refresh => {
            refresh_required()?;
            println!("Share worker refreshed");
        }
        WorkerCommand::Stop => {
            if checked_profiles()?.auto_connect {
                crate::share::ShareProfiles::mutate_persisted(Some(default_home()), |profiles| {
                    profiles.auto_connect = false;
                    Ok(())
                })?;
            }
            crate::daemon::send_share_command(crate::share::ShareCmd::Stop)?;
            println!("Share worker stopped");
        }
    }
    Ok(())
}

pub(super) fn checked_profiles() -> Result<crate::share::ShareProfiles, String> {
    crate::share::ShareProfiles::load_checked(Some(default_home()))
        .map_err(|error| format!("share profiles: {error}"))
}

pub(super) fn validate_server(server: &str) -> Result<String, String> {
    let server = server.trim();
    if server.is_empty() {
        return Err("share server must not be empty".to_string());
    }
    if server.len() > MAX_SERVER_BYTES || server.chars().any(char::is_control) {
        return Err("share server contains invalid or excessive input".to_string());
    }
    let mut found = false;
    for endpoint in server.split([',', ';']).map(str::trim) {
        if endpoint.is_empty() {
            continue;
        }
        found = true;
        if endpoint.chars().any(char::is_whitespace) || endpoint.contains('@') {
            return Err(
                "share server endpoint contains whitespace or user information".to_string(),
            );
        }
        if let Some((scheme, _)) = endpoint.split_once("://") {
            if !matches!(scheme, "tcp" | "ws" | "wss" | "http" | "https") {
                return Err(format!("unsupported share server scheme: {scheme}"));
            }
        }
    }
    if !found {
        return Err("share server must include at least one endpoint".to_string());
    }
    Ok(server.to_string())
}

fn write_atomic(path: &Path, value: &str) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    temporary
        .write_all(value.as_bytes())
        .and_then(|()| temporary.flush())
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| error.to_string())?;
    temporary
        .persist(path)
        .map(|_| ())
        .map_err(|error| error.error.to_string())
}

fn refresh_required() -> Result<(), String> {
    if crate::daemon::refresh_share_worker_checked()? {
        Ok(())
    } else {
        Err("Share server is not configured or Auto-Connect is off".to_string())
    }
}

pub(super) fn refresh_note() -> String {
    match crate::daemon::refresh_share_worker_checked() {
        Ok(true) => "; share worker refreshed".to_string(),
        Ok(false) => "; share worker inactive".to_string(),
        Err(error) => format!("; share worker refresh unavailable: {error}"),
    }
}

pub(super) fn default_home() -> String {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .to_string_lossy()
        .replace('\\', "/")
}

pub(super) fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Smart Explorer CLI".to_string())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::cli::Cli;

    const REQUEST_ID: &str = "01234567-89ab-4def-8123-456789abcdef";

    #[test]
    fn parses_durable_request_lifecycle_commands() {
        assert!(Cli::try_parse_from(["se", "share", "request"]).is_ok());
        assert!(Cli::try_parse_from(["se", "share", "request", "accept"]).is_ok());
        assert!(Cli::try_parse_from(["se", "share", "request", "list", "--json"]).is_ok());
        assert!(
            Cli::try_parse_from(["se", "share", "request", "show", REQUEST_ID, "--json"]).is_ok()
        );
        assert!(Cli::try_parse_from([
            "se",
            "share",
            "request",
            "accept",
            REQUEST_ID,
            "--fingerprint",
            "0011",
            "--message",
            "approved",
        ])
        .is_ok());
        assert!(Cli::try_parse_from(["se", "share", "request", "retry", REQUEST_ID]).is_ok());
        assert!(Cli::try_parse_from(["se", "share", "request", "delete", REQUEST_ID]).is_ok());
    }

    #[test]
    fn parses_grant_management_commands() {
        assert!(Cli::try_parse_from(["se", "share", "grants"]).is_ok());
        assert!(Cli::try_parse_from(["se", "share", "grants", "revoke"]).is_ok());
        assert!(Cli::try_parse_from(["se", "share", "grants", "list", "--json"]).is_ok());
        assert!(Cli::try_parse_from([
            "se",
            "share",
            "grants",
            "revoke",
            REQUEST_ID,
            "--fingerprint",
            "0011",
        ])
        .is_ok());
    }
}
