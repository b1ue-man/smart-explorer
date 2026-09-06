use clap::{Args, Subcommand};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::creds::{Protocol, SavedConnection};
use crate::mount::{
    BackendRoot, DriveLetter, DriveSelection, MountConfig, MountId, MountMode, MountSnapshot,
    MountSource, MountStatus, PeerMountTarget,
};

#[path = "drive_options.rs"]
mod drive_options;
use drive_options::MountArgs;

const START_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Args)]
pub(super) struct DriveArgs {
    #[command(subcommand)]
    command: DriveCommand,
}

#[derive(Subcommand)]
enum DriveCommand {
    #[command(about = "Check the required Dokany runtime and driver")]
    Runtime(OutputArgs),
    #[command(about = "Install the pinned official Dokany runtime for Windows drives")]
    InstallRuntime(InstallRuntimeArgs),
    #[command(about = "Mount a saved remote or Share peer as a Windows drive")]
    Mount(MountArgs),
    #[command(about = "List drives managed by Smart Explorer")]
    List(OutputArgs),
    #[command(about = "Unmount a managed drive by mount id or drive letter")]
    Unmount(SelectArgs),
    #[command(about = "Retry a failed managed drive")]
    Retry(SelectArgs),
}

#[derive(Args)]
struct InstallRuntimeArgs {
    #[arg(long, hide = true, value_name = "PATH")]
    msi: Option<PathBuf>,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
struct OutputArgs {
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
struct SelectArgs {
    #[arg(help = "Mount id from `se drive list`, or a mounted letter such as M:")]
    selector: String,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

pub(super) fn run(args: DriveArgs) -> Result<i32, String> {
    match args.command {
        DriveCommand::Runtime(args) => runtime(args),
        DriveCommand::InstallRuntime(args) => install_runtime(args),
        DriveCommand::Mount(args) => mount(args),
        DriveCommand::List(args) => list(args),
        DriveCommand::Unmount(args) => unmount(args),
        DriveCommand::Retry(args) => retry(args),
    }
}

fn install_runtime(args: InstallRuntimeArgs) -> Result<i32, String> {
    let outcome = crate::mount::install_drive_runtime(args.msi.as_deref())?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&outcome).map_err(|error| error.to_string())?
        );
    } else if outcome.is_failure() {
        eprintln!("dokany\tfailed\t{}", outcome.message());
    } else {
        println!("dokany\t{}", outcome.message());
    }
    Ok(outcome.exit_code())
}

fn runtime(args: OutputArgs) -> Result<i32, String> {
    let info = crate::mount::drive_runtime_info()?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&info).map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "dokany\tready\trequired={}\tlibrary={}\tdriver={}\trequired_library_api={}\tlibrary_api={}\trequired_driver_protocol={}\tdriver_protocol={}",
            info.required_api,
            info.library_api,
            info.driver_api,
            info.required_library_api,
            info.library_api,
            info.required_driver_protocol,
            info.driver_protocol
        );
    }
    Ok(0)
}

fn mount(args: MountArgs) -> Result<i32, String> {
    let (source, _display_label) = source_from_spec(&args.target)?;
    let drive = parse_drive_selection(&args.letter)?;
    let mode = if args.read_write {
        MountMode::ReadWrite
    } else {
        MountMode::ReadOnly
    };
    // A Windows volume label is visible outside Smart Explorer. Endpoint and
    // account display text therefore requires an explicit --label opt-in.
    let label = bounded_label(args.label.as_deref().unwrap_or("Smart Explorer"));
    let root_security = if args.trust_remote_root {
        crate::mount::MountRootSecurity::Trusted
    } else {
        crate::mount::MountRootSecurity::Enforced
    };
    let metadata = crate::mount::MountMetadataPolicy::new(args.metadata_depth)
        .map_err(|error| format!("invalid metadata preload policy: {error}"))?;
    let cache = crate::mount::MountCachePolicy::new(args.cache_mib)
        .map_err(|error| format!("invalid cache policy: {error}"))?;
    let runtime_preference = if args.system_runtime {
        crate::mount::MountRuntimePreference::System
    } else {
        crate::mount::MountRuntimePreference::Auto
    };
    let config = MountConfig::new(
        MountId::new_random().map_err(|error| error.to_string())?,
        source,
        drive,
        mode,
        label,
    )
    .map(|config| config.with_metadata_policy(metadata))
    .map(|config| config.with_cache_policy(cache).with_runtime_preference(runtime_preference))
    .map(|config| config.with_root_security(root_security))
    .map_err(|error| format!("invalid drive configuration: {error}"))?;
    let started = crate::daemon::start_mount(config)?;
    let snapshot = wait_until_started(started)?;
    print_snapshot(&snapshot, args.json)?;
    Ok(exit_for_status(&snapshot.status))
}

fn list(args: OutputArgs) -> Result<i32, String> {
    let mounts = crate::daemon::list_mounts()?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&mounts).map_err(|error| error.to_string())?
        );
    } else if mounts.is_empty() {
        println!("drives\t0");
    } else {
        for mount in &mounts {
            print_snapshot_text(mount);
        }
    }
    Ok(0)
}

fn unmount(args: SelectArgs) -> Result<i32, String> {
    let id = resolve_selector(&args.selector)?;
    let snapshot = crate::daemon::stop_mount(id)?;
    print_snapshot(&snapshot, args.json)?;
    Ok(exit_for_status(&snapshot.status))
}

fn retry(args: SelectArgs) -> Result<i32, String> {
    let id = resolve_selector(&args.selector)?;
    let retried = crate::daemon::retry_mount(id)?;
    let snapshot = wait_until_started(retried)?;
    print_snapshot(&snapshot, args.json)?;
    Ok(exit_for_status(&snapshot.status))
}

fn wait_until_started(mut snapshot: MountSnapshot) -> Result<MountSnapshot, String> {
    let id = snapshot.config.id.clone();
    let deadline = Instant::now() + START_TIMEOUT;
    while matches!(&snapshot.status, MountStatus::Mounting) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(150));
        snapshot = crate::daemon::list_mounts()?
            .into_iter()
            .find(|mount| mount.config.id == id)
            .ok_or_else(|| "drive disappeared while it was mounting".to_string())?;
    }
    if matches!(&snapshot.status, MountStatus::Mounting) {
        return Err(format!(
            "drive {} did not finish mounting within {} seconds",
            id,
            START_TIMEOUT.as_secs()
        ));
    }
    Ok(snapshot)
}

fn resolve_selector(selector: &str) -> Result<MountId, String> {
    let trimmed = selector.trim();
    let letter = trimmed
        .trim_end_matches([':', '\\', '/'])
        .chars()
        .collect::<Vec<_>>();
    if letter.len() == 1 && letter[0].is_ascii_alphabetic() {
        let wanted = DriveLetter::parse(letter[0]).map_err(|error| error.to_string())?;
        return crate::daemon::list_mounts()?
            .into_iter()
            .find_map(|mount| match mount.status {
                MountStatus::Mounted { drive } | MountStatus::Conflict { drive, .. }
                    if drive == wanted =>
                {
                    Some(mount.config.id)
                }
                _ => None,
            })
            .ok_or_else(|| format!("no managed drive is mounted at {wanted}"));
    }
    MountId::parse(trimmed).map_err(|error| error.to_string())
}

fn parse_drive_selection(value: &str) -> Result<DriveSelection, String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("auto") {
        return Ok(DriveSelection::Automatic);
    }
    let value = value.trim_end_matches([':', '\\', '/']);
    let mut characters = value.chars();
    let letter = characters
        .next()
        .ok_or_else(|| "drive letter is empty".to_string())?;
    if characters.next().is_some() {
        return Err("drive letter must be `auto` or one ASCII letter".into());
    }
    DriveLetter::parse(letter)
        .map(DriveSelection::Letter)
        .map_err(|error| error.to_string())
}

fn source_from_spec(spec: &str) -> Result<(MountSource, String), String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("drive source is empty".into());
    }
    if let Some((connection, path)) = super::target::saved_shorthand(spec)? {
        return saved_source(connection, &path);
    }
    if let Some((target, root)) = crate::share::PeerOpenTarget::from_endpoint(spec) {
        let label = target.endpoint_prefix();
        let target = match target {
            crate::share::PeerOpenTarget::Direct { contact_id } => {
                PeerMountTarget::Direct { contact_id }
            }
            crate::share::PeerOpenTarget::RoomDevice { room_id, device_id } => {
                PeerMountTarget::RoomDevice { room_id, device_id }
            }
        };
        return Ok((
            MountSource::Peer {
                target,
                root: parse_root(&root)?,
            },
            label,
        ));
    }
    if let Some(rest) = spec.strip_prefix("gdrive://") {
        let root = format!("/{}", rest.trim_start_matches('/'));
        return Ok((
            MountSource::GoogleDrive {
                account: "cloud:gdrive".into(),
                root: parse_root(&root)?,
            },
            "Google Drive".into(),
        ));
    }
    if crate::net::is_unc(spec) {
        return saved_unc_source(spec);
    }
    if crate::connect::is_remote_url(spec) {
        return saved_url_source(spec);
    }
    Err("only saved remotes, Google Drive, and Smart Explorer Share peers can be mounted".into())
}

fn saved_url_source(spec: &str) -> Result<(MountSource, String), String> {
    let (protocol, user, host, port, path) = crate::connect::parse_remote_url(spec)
        .ok_or_else(|| "invalid saved remote URL".to_string())?;
    let connections = crate::creds::load_connections_checked()?;
    let matches = connections.into_iter().filter(|connection| {
        connection.protocol == protocol
            && connection.user == user
            && connection.host.eq_ignore_ascii_case(&host)
            && connection.port == port
            && remote_path_is_within(&path, &connection.root)
    });
    let connection = select_longest_root(matches, false)?.ok_or_else(|| {
        "no saved connection authorizes that remote path; save the remote first".to_string()
    })?;
    saved_source(connection, &path)
}

fn saved_unc_source(spec: &str) -> Result<(MountSource, String), String> {
    let path = spec.replace('\\', "/");
    let connections = crate::creds::load_connections_checked()?;
    let matches = connections.into_iter().filter(|connection| {
        connection.protocol == Protocol::Share && unc_path_is_within(&path, &connection.root)
    });
    let connection = select_longest_root(matches, true)?.ok_or_else(|| {
        "no saved network-share connection authorizes that UNC path; save it first".to_string()
    })?;
    saved_source(connection, &path)
}

fn saved_source(connection: SavedConnection, path: &str) -> Result<(MountSource, String), String> {
    let label = connection.display();
    Ok((
        MountSource::SavedRemote {
            account: connection.account(),
            root: parse_root(path)?,
        },
        label,
    ))
}

fn select_longest_root(
    matches: impl Iterator<Item = SavedConnection>,
    unc: bool,
) -> Result<Option<SavedConnection>, String> {
    let mut matches = matches.collect::<Vec<_>>();
    matches.sort_by_key(|connection| std::cmp::Reverse(normalized(&connection.root).len()));
    let Some(longest) = matches.first() else {
        return Ok(None);
    };
    let length = normalized(&longest.root).len();
    matches.retain(|connection| normalized(&connection.root).len() == length);
    matches.dedup_by(|left, right| left.account() == right.account());
    if matches.len() > 1 {
        let accounts = matches
            .iter()
            .map(SavedConnection::account)
            .collect::<Vec<_>>()
            .join(", ");
        let kind = if unc { "network share" } else { "remote URL" };
        return Err(format!("ambiguous saved {kind}: {accounts}"));
    }
    Ok(matches.pop())
}

fn remote_path_is_within(path: &str, root: &str) -> bool {
    path_is_within(&normalized(path), &normalized(root), false)
}

fn unc_path_is_within(path: &str, root: &str) -> bool {
    path_is_within(&normalized(path), &normalized(root), true)
}

fn path_is_within(path: &str, root: &str, ignore_case: bool) -> bool {
    let (path, root) = if ignore_case {
        (path.to_lowercase(), root.to_lowercase())
    } else {
        (path.to_string(), root.to_string())
    };
    path == root
        || root == "/"
        || path
            .strip_prefix(root.trim_end_matches('/'))
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn normalized(path: &str) -> String {
    let path = path.trim().replace('\\', "/");
    if path == "/" || path == "//" {
        path
    } else {
        path.trim_end_matches('/').to_string()
    }
}

fn parse_root(path: &str) -> Result<BackendRoot, String> {
    BackendRoot::parse(&normalized(path)).map_err(|error| error.to_string())
}

fn bounded_label(label: &str) -> String {
    let label = label.trim();
    let label = if label.is_empty() {
        "Smart Explorer"
    } else {
        label
    };
    label.chars().take(128).collect()
}

fn print_snapshot(snapshot: &MountSnapshot, json: bool) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(snapshot).map_err(|error| error.to_string())?
        );
    } else {
        print_snapshot_text(snapshot);
    }
    Ok(())
}

fn print_snapshot_text(snapshot: &MountSnapshot) {
    let requested = match snapshot.config.drive {
        DriveSelection::Automatic => "auto".to_string(),
        DriveSelection::Letter(letter) => letter.to_string(),
    };
    let mode = match snapshot.config.mode {
        MountMode::ReadOnly => "read-only",
        MountMode::ReadWrite => "read-write",
    };
    println!(
        "drive\t{}\t{}\t{}\t{}\t{}",
        snapshot.config.id,
        requested,
        mode,
        status_text(&snapshot.status),
        clean(&snapshot.config.label),
    );
}

fn status_text(status: &MountStatus) -> String {
    match status {
        MountStatus::Unmounted => "unmounted".into(),
        MountStatus::Mounting => "mounting".into(),
        MountStatus::Mounted { drive } => format!("mounted:{drive}"),
        MountStatus::Unmounting => "unmounting".into(),
        MountStatus::RuntimeUnavailable { detail } => {
            format!("runtime-unavailable:{}", clean(detail))
        }
        MountStatus::Conflict { path, detail, .. } => {
            format!("conflict:{}:{}", clean(path), clean(detail))
        }
        MountStatus::Failed { detail } => format!("failed:{}", clean(detail)),
    }
}

fn exit_for_status(status: &MountStatus) -> i32 {
    if matches!(
        status,
        MountStatus::Conflict { .. }
            | MountStatus::Failed { .. }
            | MountStatus::RuntimeUnavailable { .. }
    ) {
        1
    } else {
        0
    }
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}
