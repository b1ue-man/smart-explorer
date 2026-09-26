//! Titles and breadcrumbs of a location (`fs.list` → `title`, `crumbs`).
use super::config::VolumeInfo;
use super::location::{is_same_or_below, join, Loc, LocKind};
use serde_json::{json, Value};

/// The volume that contains `path` (the deepest one when volumes nest).
pub(crate) fn volume_of<'a>(path: &str, volumes: &'a [VolumeInfo]) -> Option<&'a VolumeInfo> {
    volumes
        .iter()
        .filter(|volume| is_same_or_below(path, &volume.path))
        .max_by_key(|volume| volume.path.trim_end_matches('/').len())
}

/// True when `path` is exactly a volume root.
pub(crate) fn is_volume_root(path: &str, volumes: &[VolumeInfo]) -> bool {
    volumes
        .iter()
        .any(|volume| volume.path.trim_end_matches('/') == path.trim_end_matches('/'))
}

/// The label of a connection root (`user@host:port`, `Google Drive`, …).
pub(crate) fn root_label(loc: &Loc) -> String {
    match loc.kind {
        LocKind::Local => "Gerät".to_string(),
        LocKind::GDrive => "Google Drive".to_string(),
        LocKind::Share => "Share".to_string(),
        LocKind::Trash => "Papierkorb".to_string(),
        LocKind::Zip => loc
            .zip_archive()
            .and_then(|archive| archive.rsplit('/').next())
            .unwrap_or("ZIP")
            .to_string(),
        LocKind::Sftp | LocKind::Ftp | LocKind::Ftps | LocKind::Webdav | LocKind::Smb => loc
            .prefix
            .split_once("://")
            .map(|(_, authority)| authority.to_string())
            .unwrap_or_else(|| loc.prefix.clone()),
    }
}

/// The page title: the folder name, a volume label or the connection label.
pub(crate) fn title(loc: &Loc, volumes: &[VolumeInfo]) -> String {
    if loc.kind == LocKind::Local {
        if let Some(volume) = volumes
            .iter()
            .find(|volume| volume.path.trim_end_matches('/') == loc.path)
        {
            return volume_label(volume);
        }
    }
    if loc.is_root() {
        let label = root_label(loc);
        return if loc.kind == LocKind::Zip {
            format!("{label} (nur lesen)")
        } else {
            label
        };
    }
    loc.name().to_string()
}

fn volume_label(volume: &VolumeInfo) -> String {
    if volume.label.trim().is_empty() {
        volume.path.clone()
    } else {
        volume.label.clone()
    }
}

/// `[{label, location}]` from the root of the location's volume/connection.
pub(crate) fn crumbs(loc: &Loc, volumes: &[VolumeInfo]) -> Vec<Value> {
    if loc.kind == LocKind::Trash {
        return vec![json!({ "label": "Papierkorb", "location": loc.location() })];
    }
    let (first_label, first_path) = match volume_of(&loc.path, volumes) {
        Some(volume) if loc.kind == LocKind::Local => (
            volume_label(volume),
            volume.path.trim_end_matches('/').to_string(),
        ),
        _ => (root_label(loc), "/".to_string()),
    };
    let first_path = if first_path.is_empty() {
        "/".to_string()
    } else {
        first_path
    };
    let mut crumbs = vec![json!({ "label": first_label, "location": loc.at(&first_path) })];
    let rest = loc
        .path
        .strip_prefix(first_path.trim_end_matches('/'))
        .unwrap_or("");
    let mut current = first_path;
    for segment in rest.split('/').filter(|segment| !segment.is_empty()) {
        current = join(&current, segment);
        crumbs.push(json!({ "label": segment, "location": loc.at(&current) }));
    }
    crumbs
}
