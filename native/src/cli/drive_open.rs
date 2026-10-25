//! `se drive open <SELEKTOR> [--json]` (DRV-M5, contract V4): shows a
//! mounted drive in the file manager. Linux asks the file manager of the
//! session (`FileManager1`, else `xdg-open`), Windows starts Explorer on the
//! letter.
//!
//! Text: `opened\t<Pfad>\t<Methode>`. JSON: `{"opened":"<Pfad>","method":
//! "filemanager1|xdg-open|explorer"}`. A drive that is not mounted prints
//! `se: Das Laufwerk ist nicht eingehängt` and exits with 1. Through MCP the
//! command runs with the `se` tool.
use std::io;
use std::path::{Path, PathBuf};

use clap::Args;
use clap_complete::ArgValueCandidates;

use crate::mount::{MountSnapshot, MountStatus};

#[derive(Args)]
pub(super) struct OpenArgs {
    #[arg(
        help = "Mount id from `se drive list`, `devices`, a mounted letter such as M: or a mount folder",
        add = ArgValueCandidates::new(super::drive_set::selector_candidates)
    )]
    selector: String,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

pub(super) fn open(args: OpenArgs) -> Result<i32, String> {
    let id = super::resolve_selector(&args.selector)?;
    let snapshot = crate::daemon::list_mounts()?
        .into_iter()
        .find(|mount| mount.config.id == id)
        .ok_or_else(|| format!("unknown drive {id}"))?;
    match open_with(&snapshot, &system_opener) {
        Ok((opened, method)) => {
            if args.json {
                let value = serde_json::json!({ "opened": opened, "method": method });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
                );
            } else {
                println!("opened\t{}\t{method}", super::drive_print::clean(&opened));
            }
            Ok(0)
        }
        Err(message) => {
            eprintln!("se: {message}");
            Ok(1)
        }
    }
}

/// The folder of a mounted drive and how `opener` showed it.
fn open_with(
    snapshot: &MountSnapshot,
    opener: &dyn Fn(&Path) -> io::Result<&'static str>,
) -> Result<(String, &'static str), String> {
    let path = match &snapshot.status {
        MountStatus::Mounted { drive } => PathBuf::from(format!("{drive}\\")),
        MountStatus::MountedAt { path } => path.clone(),
        _ => return Err("Das Laufwerk ist nicht eingehängt".into()),
    };
    let method = opener(&path).map_err(|error| error.to_string())?;
    Ok((path.to_string_lossy().into_owned(), method))
}

#[cfg(target_os = "linux")]
fn system_opener(path: &Path) -> io::Result<&'static str> {
    crate::mount::os::linux_os::show_folders(&[path.to_path_buf()]).map(|method| method.as_str())
}

/// Explorer is not waited for; its exit code says nothing.
#[cfg(windows)]
fn system_opener(path: &Path) -> io::Result<&'static str> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map(|_| "explorer")
}

#[cfg(not(any(windows, target_os = "linux")))]
fn system_opener(_path: &Path) -> io::Result<&'static str> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Laufwerke gibt es nur unter Windows und Linux",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mount::{
        DriveLetter, DriveSelection, MountConfig, MountId, MountMode, MountRecovery, MountSource,
    };

    fn snapshot(status: MountStatus) -> MountSnapshot {
        let config = MountConfig::new(
            MountId::parse("open-test").unwrap(),
            MountSource::Devices,
            DriveSelection::Automatic,
            MountMode::ReadWrite,
            "Geräte",
        )
        .unwrap();
        MountSnapshot {
            capacity: None,
            config,
            status,
            recovery: MountRecovery::Clean,
            recovery_required_compat: false,
        }
    }

    #[test]
    fn drive_task_cli_open_json() {
        let folder = snapshot(MountStatus::MountedAt {
            path: PathBuf::from("/home/u/SmartExplorer/Geräte"),
        });
        let (opened, method) = open_with(&folder, &|_| Ok("filemanager1")).unwrap();
        let value = serde_json::json!({ "opened": opened, "method": method });
        let printed = serde_json::to_string_pretty(&value).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&printed).unwrap();
        assert_eq!(parsed["opened"], "/home/u/SmartExplorer/Geräte");
        assert_eq!(parsed["method"], "filemanager1");
        assert_eq!(parsed.as_object().map(|object| object.len()), Some(2));
        let letter = snapshot(MountStatus::Mounted {
            drive: DriveLetter::parse('x').unwrap(),
        });
        let (opened, method) = open_with(&letter, &|_| Ok("explorer")).unwrap();
        assert_eq!((opened.as_str(), method), ("X:\\", "explorer"));
        // The opener sees the folder; its failure is the message.
        let failed = open_with(&folder, &|path| {
            assert!(path.ends_with("Geräte"));
            Err(io::Error::other("kein Dateimanager"))
        });
        assert_eq!(failed.unwrap_err(), "kein Dateimanager");
        let unmounted = open_with(&snapshot(MountStatus::Unmounted), &|_| Ok("xdg-open"));
        assert_eq!(unmounted.unwrap_err(), "Das Laufwerk ist nicht eingehängt");
    }
}
