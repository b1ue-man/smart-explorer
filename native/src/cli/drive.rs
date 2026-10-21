use clap::{Args, Subcommand};
use clap_complete::ArgValueCandidates;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::mount::{
    DriveLetter, DriveSelection, MountConfig, MountId, MountMode, MountSnapshot, MountSource,
    MountStatus,
};

#[path = "drive_options.rs"]
mod drive_options;
use drive_options::{absolute_folder, source_from_spec, MountArgs, UnmountArgs};
#[path = "drive_devices.rs"]
mod drive_devices;
#[path = "drive_open.rs"]
mod drive_open;
#[path = "drive_print.rs"]
mod drive_print;
use drive_print::{exit_for_status, print_snapshot, print_snapshot_text};
#[cfg(test)]
use drive_print::{snapshot_line, status_column};
#[path = "drive_set.rs"]
mod drive_set;
// DRV-M4: runtime, install answer, folder column and unmount under Linux.
#[cfg(target_os = "linux")]
#[path = "drive_output_linux.rs"]
mod drive_output_linux;

const START_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Args)]
pub(super) struct DriveArgs {
    #[command(subcommand)]
    command: DriveCommand,
}

#[derive(Subcommand)]
enum DriveCommand {
    #[command(about = "Check the drive runtime (Windows: Dokany, Linux: FUSE)")]
    Runtime(OutputArgs),
    #[command(
        about = "Install the pinned official Dokany runtime (Windows); name the FUSE package (Linux)"
    )]
    InstallRuntime(InstallRuntimeArgs),
    #[command(
        about = "Mount a saved remote, Share peer or unlocked vault (vault://<SITE>) as a drive (Windows: letter, Linux: folder)"
    )]
    Mount(MountArgs),
    #[command(about = "List drives managed by Smart Explorer")]
    List(OutputArgs),
    #[command(about = "Unmount a managed drive by mount id, drive letter or folder")]
    Unmount(UnmountArgs),
    #[command(about = "Retry a failed managed drive")]
    Retry(SelectArgs),
    #[command(about = "Change options of a managed drive, such as mounting it again at login")]
    Set(drive_set::SetArgs),
    #[command(about = "Open a mounted drive in the file manager")]
    Open(drive_open::OpenArgs),
    #[command(about = "Show or switch the device tree (connected devices as folders)")]
    Devices(drive_devices::DevicesArgs),
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
    #[arg(
        help = "Mount id from `se drive list`, or a mounted letter such as M:",
        add = ArgValueCandidates::new(drive_set::selector_candidates)
    )]
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
        DriveCommand::Set(args) => drive_set::set(args),
        DriveCommand::Open(args) => drive_open::open(args),
        DriveCommand::Devices(args) => drive_devices::run(args),
    }
}

#[cfg(target_os = "linux")]
fn install_runtime(args: InstallRuntimeArgs) -> Result<i32, String> {
    // se installs no system package under Linux; `--msi` is Windows only.
    let _ = args.msi;
    drive_output_linux::install_runtime(args.json)
}

#[cfg(not(target_os = "linux"))]
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

#[cfg(target_os = "linux")]
fn runtime(args: OutputArgs) -> Result<i32, String> {
    drive_output_linux::runtime(args.json)
}

#[cfg(not(target_os = "linux"))]
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
    #[cfg(target_os = "linux")]
    if let Some(refusal) =
        drive_output_linux::refuse_windows_options(&args.letter, args.system_runtime)
    {
        eprintln!("se: {refusal}");
        return Ok(2);
    }
    #[cfg(not(target_os = "linux"))]
    if args.mount_point.is_some() {
        eprintln!("se: Einhängeorte gibt es nur unter Linux; nutze --letter");
        return Ok(2);
    }
    let folder = args
        .mount_point
        .as_deref()
        .map(absolute_folder)
        .transpose()?;
    // DRV-M7 (P3): a vault mounts as `vault://<SITE>`; its label is the
    // vault's label unless `--label` names another.
    let vault = vault_source(&args.target)?;
    let (source, _display_label) = match &vault {
        Some((source, label)) => (source.clone(), label.clone()),
        None => source_from_spec(&args.target)?,
    };
    let drive = parse_drive_selection(&args.letter)?;
    let mode = if args.read_write {
        MountMode::ReadWrite
    } else {
        MountMode::ReadOnly
    };
    // A Windows volume label is visible outside Smart Explorer. Endpoint and
    // account display text therefore requires an explicit --label opt-in;
    // a vault's own label is the name the user gave it.
    let default_label = vault
        .as_ref()
        .map_or("Smart Explorer", |(_, label)| label.as_str());
    let label = bounded_label(args.label.as_deref().unwrap_or(default_label));
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
    .map(|config| {
        config
            .with_cache_policy(cache)
            .with_runtime_preference(runtime_preference)
    })
    .map(|config| config.with_root_security(root_security))
    .map(|config| config.with_auto_remount(args.auto_remount))
    .and_then(|config| config.with_location_directory(folder))
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

fn unmount(args: UnmountArgs) -> Result<i32, String> {
    let id = resolve_selector(&args.selector)?;
    // DRV-M6: the device tree is switched off; that also forgets it when
    // unsaved changes of devices remain (their journals stay).
    if id.as_str() == crate::mount::DEVICES_MOUNT_ID {
        return drive_devices::print_status(&crate::daemon::devices_disable()?, args.json);
    }
    // Linux: the folder is unmounted first, like a file manager's eject; a
    // busy drive stays mounted unless --force (DRV-M4).
    #[cfg(target_os = "linux")]
    let before = drive_output_linux::release_before_stop(&id, args.force)?;
    #[cfg(not(target_os = "linux"))]
    let _ = args.force;
    let snapshot = match crate::daemon::stop_mount(id) {
        Ok(snapshot) => snapshot,
        #[cfg(target_os = "linux")]
        Err(error) => drive_output_linux::stopped_meanwhile(before, error)?,
        #[cfg(not(target_os = "linux"))]
        Err(error) => return Err(error),
    };
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
    // DRV-M6: `devices` names the device tree when it is switched on.
    if trimmed == "devices"
        && crate::daemon::list_mounts()?
            .iter()
            .any(|mount| mount.config.source.is_device_tree())
    {
        return MountId::parse(crate::mount::DEVICES_MOUNT_ID).map_err(|error| error.to_string());
    }
    #[cfg(target_os = "linux")]
    if let Some(id) = drive_output_linux::select_by_folder(trimmed)? {
        return Ok(id);
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

fn bounded_label(label: &str) -> String {
    let label = label.trim();
    let label = if label.is_empty() {
        "Smart Explorer"
    } else {
        label
    };
    label.chars().take(128).collect()
}

/// `vault://<SITE>` as a drive source: the site ID of the vault list and
/// the vault's label. A vault drive always shows the whole vault.
fn vault_source(target: &str) -> Result<Option<(MountSource, String)>, String> {
    let target = target.trim();
    if !crate::vault::is_vault_locator(target) {
        return Ok(None);
    }
    let locator = crate::vault::VaultLocator::parse(target).map_err(|_| {
        "vault-store:// lässt sich nicht einbinden; nutze vault://<SITE>".to_string()
    })?;
    if locator.path.trim_matches('/') != "" {
        return Err("Ein Tresor-Laufwerk zeigt immer den ganzen Tresor: vault://<SITE>".into());
    }
    let record = crate::vault::RegistryStore::open_default().resolve(&locator.site)?;
    let label = record.display_label().to_string();
    Ok(Some((MountSource::Vault { site_id: record.id }, label)))
}

#[cfg(test)]
mod vault_status_tests {
    use super::{snapshot_line, status_column};
    use crate::mount::{
        DriveSelection, MountConfig, MountId, MountMode, MountRecovery, MountSnapshot, MountSource,
        MountStatus,
    };

    fn vault_drive(status: MountStatus, recovery: MountRecovery) -> MountSnapshot {
        let config = MountConfig::new(
            MountId::parse("vault-drive").unwrap(),
            MountSource::Vault {
                site_id: "0123456789abcdef0123456789abcdef".into(),
            },
            DriveSelection::Automatic,
            MountMode::ReadWrite,
            "Privat",
        )
        .unwrap();
        MountSnapshot {
            config,
            status,
            recovery,
            recovery_required_compat: recovery.requires_retention(),
        }
    }

    #[test]
    fn vault_drive_cli_status_columns() {
        let locked = MountStatus::Failed {
            detail: "vault_locked: Tresor Privat ist gesperrt: se vault unlock 0123".into(),
        };
        let clean = vault_drive(locked.clone(), MountRecovery::Clean);
        assert_eq!(status_column(&clean), "failed:vault_locked");
        assert!(snapshot_line(&clean).contains("\tfailed:vault_locked\t"));
        let pending = vault_drive(locked, MountRecovery::RequiresUnlock { pending: 3 });
        assert_eq!(status_column(&pending), "requires-unlock:3");
        let json = serde_json::to_value(&pending).unwrap();
        assert_eq!(json["recovery"]["requires_unlock"]["pending"], 3);
        assert_eq!(json["recovery_required"], true);
        // Other failures keep their text.
        let other = vault_drive(
            MountStatus::Failed {
                detail: "vault_lockedness".into(),
            },
            MountRecovery::Clean,
        );
        assert_eq!(status_column(&other), "failed:vault_lockedness");
    }
}

// DRV-M10: contracts of the drive commands that earlier releases set (stage
// names of a mounted save, JSON form of `MountSnapshot`).
#[cfg(test)]
#[path = "drive_contract_tests.rs"]
mod drive_contract_tests;

// DRV-M10: Node's `realpath` on a real Dokany drive (Windows task stage).
#[cfg(all(test, windows))]
#[path = "drive_realpath_task_tests.rs"]
mod drive_realpath_task_tests;
