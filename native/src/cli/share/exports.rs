use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Args)]
pub(super) struct ExportArgs {
    #[command(subcommand)]
    command: ExportCommand,
}

#[derive(Subcommand)]
enum ExportCommand {
    List(ExportListArgs),
    Add(ExportAddArgs),
    Remove(ExportRemoveArgs),
}

#[derive(Args)]
struct ExportScope {
    #[arg(long, help = "Exact room profile id, room id, or room name")]
    room: Option<String>,
}

#[derive(Args)]
struct ExportListArgs {
    #[command(flatten)]
    scope: ExportScope,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Args)]
struct ExportAddArgs {
    #[arg(help = "Existing local directory to export")]
    path: PathBuf,
    #[arg(long, default_value = "", help = "Peer-visible export label")]
    label: String,
    #[command(flatten)]
    scope: ExportScope,
}

#[derive(Args)]
struct ExportRemoveArgs {
    #[arg(help = "Exact export label or canonical path")]
    selector: String,
    #[command(flatten)]
    scope: ExportScope,
}

pub(super) fn run(args: ExportArgs) -> Result<(), String> {
    match args.command {
        ExportCommand::List(list) => list_exports(list),
        ExportCommand::Add(add) => add_export(add),
        ExportCommand::Remove(remove) => remove_export(remove),
    }
}

fn list_exports(args: ExportListArgs) -> Result<(), String> {
    let profiles = super::checked_profiles()?;
    let config = export_config(&profiles, args.scope.room.as_deref())?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(config).map_err(|error| error.to_string())?
        );
    } else {
        println!("include_connections\t{}", config.include_connections);
        for root in &config.roots {
            println!("export\t{}\t{}", root.label, root.path);
        }
    }
    Ok(())
}

fn add_export(args: ExportAddArgs) -> Result<(), String> {
    let path = canonical_directory(&args.path)?;
    let label = export_label(&path, &args.label);
    let room = args.scope.room;
    crate::share::ShareProfiles::mutate_persisted(Some(super::default_home()), |profiles| {
        let config = export_config_mut(profiles, room.as_deref())?;
        if config.roots.iter().any(|root| root.path == path) {
            return Err(format!("export already exists: {path}"));
        }
        config.roots.push(crate::share::SharedRoot {
            label: label.clone(),
            path: path.clone(),
        });
        Ok(())
    })?;
    println!("Added export {path}{}", super::refresh_note());
    Ok(())
}

fn remove_export(args: ExportRemoveArgs) -> Result<(), String> {
    let selector = args.selector.trim();
    if selector.is_empty() {
        return Err("export selector must not be empty".to_string());
    }
    let room = args.scope.room;
    let mut removed_path = None;
    crate::share::ShareProfiles::mutate_persisted(Some(super::default_home()), |profiles| {
        let config = export_config_mut(profiles, room.as_deref())?;
        let matches = config
            .roots
            .iter()
            .enumerate()
            .filter(|(_, root)| root.label == selector || root.path == selector)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(if matches.is_empty() {
                format!("export not found: {selector}")
            } else {
                format!("export selector is ambiguous: {selector}")
            });
        }
        removed_path = Some(config.roots.remove(matches[0]).path);
        Ok(())
    })?;
    let removed_path =
        removed_path.ok_or_else(|| "export removal was not committed".to_string())?;
    println!("Removed export {removed_path}{}", super::refresh_note());
    Ok(())
}

fn export_config<'a>(
    profiles: &'a crate::share::ShareProfiles,
    room: Option<&str>,
) -> Result<&'a crate::share::ShareExportConfig, String> {
    match room {
        None => Ok(&profiles.default_direct_exports),
        Some(selector) => {
            room_index(profiles, selector).map(|index| &profiles.rooms[index].exports)
        }
    }
}

fn export_config_mut<'a>(
    profiles: &'a mut crate::share::ShareProfiles,
    room: Option<&str>,
) -> Result<&'a mut crate::share::ShareExportConfig, String> {
    match room {
        None => Ok(&mut profiles.default_direct_exports),
        Some(selector) => {
            let index = room_index(profiles, selector)?;
            Ok(&mut profiles.rooms[index].exports)
        }
    }
}

fn room_index(profiles: &crate::share::ShareProfiles, selector: &str) -> Result<usize, String> {
    let matches = profiles
        .rooms
        .iter()
        .enumerate()
        .filter(|(_, room)| {
            room.id == selector || room.room_id == selector || room.name == selector
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(format!("room not found: {selector}")),
        [index] => Ok(*index),
        _ => Err(format!("room selector is ambiguous: {selector}")),
    }
}

fn canonical_directory(path: &Path) -> Result<String, String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("export path {}: {error}", path.display()))?;
    if !canonical
        .metadata()
        .map_err(|error| error.to_string())?
        .is_dir()
    {
        return Err(format!(
            "export path is not a directory: {}",
            path.display()
        ));
    }
    Ok(canonical.to_string_lossy().replace('\\', "/"))
}

fn export_label(path: &str, requested: &str) -> String {
    let requested = requested.trim();
    if !requested.is_empty() {
        return requested.to_string();
    }
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Export")
        .to_string()
}
