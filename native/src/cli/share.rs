use clap::{Args, Subcommand};
use std::io::Write;
use std::path::{Path, PathBuf};

mod exports;
#[path = "share/identity_command.rs"]
mod identity_command;

const MAX_SERVER_BYTES: usize = 16 * 1024;

#[derive(Args)]
pub(super) struct ShareArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Configure and start the headless Share worker")]
    Configure(ConfigureArgs),
    #[command(about = "Show this device's Share identity and direct invite code")]
    Identity(identity_command::IdentityArgs),
    #[command(about = "Show worker, peer, room, and pending-request status")]
    Status(JsonArgs),
    #[command(about = "Accept or reject a pending direct access request")]
    Request(RequestArgs),
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
struct JsonArgs {
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
struct RequestArgs {
    #[command(subcommand)]
    command: RequestCommand,
}

#[derive(Subcommand)]
enum RequestCommand {
    Accept(AnswerArgs),
    Reject(AnswerArgs),
}

#[derive(Args)]
struct AnswerArgs {
    #[arg(help = "Exact device id from `se share status`")]
    device_id: String,
    #[arg(long, help = "Exact fingerprint shown by `se share status`")]
    fingerprint: String,
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
        Command::Configure(args) => configure(args)?,
        Command::Identity(args) => identity_command::run(args)?,
        Command::Status(args) => status(args.json)?,
        Command::Request(args) => match args.command {
            RequestCommand::Accept(answer) => answer_request(answer, true)?,
            RequestCommand::Reject(answer) => answer_request(answer, false)?,
        },
        Command::Export(args) => exports::run(args)?,
        Command::Room(args) => match args.command {
            RoomCommand::Create { name } => create_room(&name)?,
        },
        Command::Worker(args) => worker(args.command)?,
    }
    Ok(0)
}

fn configure(args: ConfigureArgs) -> Result<(), String> {
    let server = validate_server(&args.server)?;
    let mut identity = identity_command::load_with_repair_hint()?;
    if let Some(name) = args.device_name {
        identity.set_device_name(name)?;
    }
    let mut profiles = checked_profiles()?;
    if !profiles.auto_connect {
        let mut candidate = profiles.clone();
        candidate.auto_connect = true;
        profiles.persist_replacement(candidate)?;
    }
    write_atomic(
        &crate::support_dirs::app_data_file("share_server.txt"),
        &server,
    )?;
    refresh_required()?;
    println!("Share worker configured for {server}");
    Ok(())
}

fn status(json: bool) -> Result<(), String> {
    let snapshot = crate::daemon::drain_share_worker_events()?;
    let profiles = checked_profiles()?;
    if json {
        let contacts = profiles.direct_contacts.iter().map(|contact| {
            serde_json::json!({
                "id": contact.id,
                "name": contact.display_name,
                "status": contact.status.label(),
                "access": contact.access_state.label(),
                "fingerprint": contact.expected_fingerprint,
            })
        });
        let rooms = profiles.rooms.iter().map(|room| {
            serde_json::json!({
                "id": room.id,
                "room_id": room.room_id,
                "name": room.name,
                "status": room.status.label(),
                "members": room.members.len(),
            })
        });
        let pending = snapshot.pending_direct_requests.iter().map(|request| {
            serde_json::json!({
                "device_id": request.device_id,
                "device_name": request.device_name,
                "fingerprint": request.fingerprint,
            })
        });
        let value = serde_json::json!({
            "running": snapshot.running,
            "connected": snapshot.connected,
            "last_error": snapshot.last_error,
            "relay_url": snapshot.relay_url,
            "candidates": snapshot.candidates,
            "contacts": contacts.collect::<Vec<_>>(),
            "rooms": rooms.collect::<Vec<_>>(),
            "pending_requests": pending.collect::<Vec<_>>(),
            "events": snapshot.events.iter().filter_map(public_event).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    println!("running\t{}", snapshot.running);
    println!("connected\t{}", snapshot.connected);
    println!(
        "last_error\t{}",
        snapshot.last_error.as_deref().unwrap_or("-")
    );
    println!("relay_url\t{}", snapshot.relay_url);
    for contact in &profiles.direct_contacts {
        println!(
            "peer\t{}\t{}\t{}\t{}",
            contact.id,
            contact.display_name,
            contact.status.label(),
            contact.access_state.label()
        );
    }
    for room in &profiles.rooms {
        println!(
            "room\t{}\t{}\t{}\t{}",
            room.id,
            room.name,
            room.status.label(),
            room.members.len()
        );
    }
    for request in &snapshot.pending_direct_requests {
        println!(
            "request\t{}\t{}\t{}",
            request.device_id, request.device_name, request.fingerprint
        );
    }
    for event in snapshot.events.iter().filter_map(public_event) {
        println!("event\t{event}");
    }
    Ok(())
}

fn public_event(event: &crate::share::ShareEvent) -> Option<String> {
    match event {
        crate::share::ShareEvent::Status(message) => Some(format!("status: {message}")),
        crate::share::ShareEvent::Error(message) => Some(format!("error: {message}")),
        crate::share::ShareEvent::ServerConnected => Some("server connected".to_string()),
        crate::share::ShareEvent::ServerDisconnected(message) => {
            Some(format!("server disconnected: {message}"))
        }
        _ => None,
    }
}

fn answer_request(args: AnswerArgs, accepted: bool) -> Result<(), String> {
    let snapshot = crate::daemon::drain_share_worker_events()?;
    let mut matches = snapshot
        .pending_direct_requests
        .into_iter()
        .filter(|request| request.device_id == args.device_id)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(if matches.is_empty() {
            format!("pending request not found: {}", args.device_id)
        } else {
            format!("pending request id is ambiguous: {}", args.device_id)
        });
    }
    let request = matches.remove(0);
    if request.expires_at < crate::share::core_now_secs() {
        return Err(format!("pending request expired: {}", args.device_id));
    }
    if !request
        .fingerprint
        .eq_ignore_ascii_case(args.fingerprint.trim())
    {
        return Err(format!(
            "fingerprint mismatch for {}: expected {}",
            args.device_id, request.fingerprint
        ));
    }
    let identity = crate::share::ShareIdentity::load_or_create(default_device_name())?;
    let mut profiles = checked_profiles()?;
    let state = if accepted {
        crate::share::DirectGrantState::Accepted
    } else {
        crate::share::DirectGrantState::Ignored
    };
    profiles.set_direct_grant_persisted(&request, state)?;
    crate::daemon::send_share_command(crate::share::ShareCmd::AnswerDirectRequest {
        lookup_id: identity.direct_lookup_id,
        presence: request,
        accepted,
    })
    .map_err(|error| format!("decision persisted, but response delivery failed: {error}"))?;
    println!(
        "{} request from {}",
        if accepted { "Accepted" } else { "Rejected" },
        args.device_id
    );
    Ok(())
}

fn create_room(name: &str) -> Result<(), String> {
    let code = crate::share::ShareProfiles::new_room_code()?;
    let mut profiles = checked_profiles()?;
    let id = profiles.add_room_from_code(&code, name)?;
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
            let mut profiles = checked_profiles()?;
            if profiles.auto_connect {
                let mut candidate = profiles.clone();
                candidate.auto_connect = false;
                profiles.persist_replacement(candidate)?;
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

fn default_home() -> String {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .to_string_lossy()
        .replace('\\', "/")
}

fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Smart Explorer CLI".to_string())
}
