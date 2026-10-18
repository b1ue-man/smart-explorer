//! Mount creation options, shared policy defaults and typed CLI validation:
//! the arguments of `mount` and `unmount`, the drive source a `<TARGET>`
//! names (moved out of `drive.rs` before DRV-M4 extended it), and the folder
//! of `--mount-point` (Linux, DRV-M4).

use clap::Args;
use clap_complete::ArgValueCandidates;
use std::path::PathBuf;

use crate::creds::{Protocol, SavedConnection};
use crate::mount::{BackendRoot, MountConfig, MountSource, PeerMountTarget};

#[derive(Args)]
pub(super) struct MountArgs {
    #[arg(help = "Saved @label:/path, remote URL, gdrive:// path, or share:// endpoint")]
    pub(super) target: String,
    #[arg(
        long,
        default_value = "auto",
        value_name = "AUTO|LETTER",
        help = "Windows: choose a drive letter automatically or request one letter"
    )]
    pub(super) letter: String,
    #[arg(
        long,
        value_name = "PATH",
        value_hint = clap::ValueHint::DirPath,
        help = "Linux: the empty folder the drive appears at; default ~/SmartExplorer/<label>"
    )]
    pub(super) mount_point: Option<PathBuf>,
    #[arg(
        long,
        help = "Enable writes; the default is read-only and safer for remote protocols"
    )]
    pub(super) read_write: bool,
    #[arg(
        long,
        help = "Trust the selected root when the active backend cannot provide race-proof root confinement"
    )]
    pub(super) trust_remote_root: bool,
    #[arg(
        long,
        default_value_t = crate::mount::DEFAULT_METADATA_PRELOAD_DEPTH,
        value_parser = clap::value_parser!(u8).range(0..=4),
        help = "Preload directory metadata to this depth; 0 caches only opened folders"
    )]
    pub(super) metadata_depth: u8,
    #[arg(
        long,
        default_value_t = crate::mount::DEFAULT_MOUNT_CACHE_MIB,
        value_parser = clap::value_parser!(u32).range(0..=i64::from(crate::mount::MAX_MOUNT_CACHE_MIB)),
        value_name = "MIB",
        help = "Retain up to this many MiB of idle clean files; 0 disables retention. Open/unsaved data is excluded"
    )]
    pub(super) cache_mib: u32,
    #[arg(
        long,
        help = "Windows: use the official System32 Dokany DLL for compatibility instead of the optimized private runtime"
    )]
    pub(super) system_runtime: bool,
    #[arg(
        long,
        value_name = "TEXT",
        help = "Name of the drive: Windows volume label, Linux folder name"
    )]
    pub(super) label: Option<String>,
    #[arg(
        long,
        help = "Mount this drive again at every login and after a background-worker restart"
    )]
    pub(super) auto_remount: bool,
    #[arg(long, help = "Print machine-readable JSON")]
    pub(super) json: bool,
}

#[derive(Args)]
pub(super) struct UnmountArgs {
    #[arg(
        help = "Mount id from `se drive list`, a mounted letter such as M:, or the folder (Linux)",
        add = ArgValueCandidates::new(super::drive_set::selector_candidates)
    )]
    pub(super) selector: String,
    #[arg(
        long,
        help = "Linux: unmount a busy or hanging drive anyway (open files lose their connection)"
    )]
    pub(super) force: bool,
    #[arg(long, help = "Print machine-readable JSON")]
    pub(super) json: bool,
}

/// `--mount-point` relative to the working folder; the form is checked here,
/// owner and content by the host right before it mounts.
pub(super) fn absolute_folder(path: &std::path::Path) -> Result<PathBuf, String> {
    let folder = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("working folder: {error}"))?
            .join(path)
    };
    MountConfig::check_location_directory(&folder)
        .map(|()| folder)
        .map_err(|error| format!("invalid --mount-point: {error}"))
}

pub(super) fn source_from_spec(spec: &str) -> Result<(MountSource, String), String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("drive source is empty".into());
    }
    if let Some((connection, path)) = super::super::target::saved_shorthand(spec)? {
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

#[cfg(test)]
mod optimization_task {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Command {
        #[command(flatten)]
        mount: MountArgs,
    }

    #[test]
    fn mount_optimization_task_cli_cache_and_runtime_policy() {
        let defaults = Command::try_parse_from(["mount", "@fixture:/vault"]).unwrap();
        assert_eq!(defaults.mount.cache_mib, 500);
        assert!(!defaults.mount.system_runtime);
        assert!(!defaults.mount.auto_remount);
        for value in ["0", "1", "500", "65536"] {
            let command = Command::try_parse_from([
                "mount",
                "@fixture:/vault",
                "--cache-mib",
                value,
                "--system-runtime",
            ])
            .unwrap();
            assert_eq!(command.mount.cache_mib, value.parse::<u32>().unwrap());
            assert!(command.mount.system_runtime);
        }
        for value in ["-1", "65537", "4294967296", "many"] {
            assert!(
                Command::try_parse_from(["mount", "@fixture:/vault", "--cache-mib", value,])
                    .is_err()
            );
        }
    }
}
