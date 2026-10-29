//! Build and persist removal proof using fixed installation paths and hashes.
use crate::mcp_launcher::installation::{hash_bytes, read_json};
use crate::mcp_launcher::{os, Installation, Manifest};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RemovalFile {
    pub(super) path: String,
    pub(super) sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RemovalRecord {
    schema: u32,
    pub(super) manifest: Manifest,
    pub(super) files: Vec<RemovalFile>,
    pub(super) directories: Vec<String>,
}

pub(super) fn load_removal(root: &Path) -> Result<Option<RemovalRecord>, String> {
    let path = root.join("mcp/uninstall.v1.json");
    if !exists(&path)? {
        return Ok(None);
    }
    os::validate_tree(root, &path)?;
    let record: RemovalRecord = read_json(&path)?;
    record.manifest.validate(os::CLI_NAME)?;
    let mut names = std::collections::BTreeSet::new();
    if record.schema != 1
        || record.files.iter().any(|file| {
            !removal_file_allowed(&file.path)
                || !crate::mcp_launcher::core::manifest::is_sha256(&file.sha256)
                || !names.insert(file.path.clone())
        })
        || record
            .directories
            .iter()
            .any(|path| !removal_directory_allowed(path))
        || [
            "mcp/current.v1.json",
            "mcp/installation.id",
            "mcp/update-owner.lock",
            "mcp/control.lock",
            "mcp/operations.lock",
        ]
        .iter()
        .any(|name| !names.contains(*name))
    {
        return Err("uninstall_record_invalid".into());
    }
    Ok(Some(record))
}

pub(super) fn write_removal_record(root: &Path, record: &RemovalRecord) -> Result<(), String> {
    // Match read_json's existing budget before exposing an unrecoverable record.
    let bytes = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    if bytes.len() > 256 * 1024 {
        return Err("uninstall_record_exceeds_record_budget: owned files retained".into());
    }
    crate::mcp_launcher::installation::atomic_bytes(&root.join("mcp/uninstall.v1.json"), &bytes)
}

pub(super) fn inventory(installation: &Installation) -> Result<RemovalRecord, String> {
    let root = &installation.root;
    let id = &installation.manifest.installation_id;
    let mut files = Vec::new();
    let mut directories = Vec::new();
    for category in ["runtimes", "launchers"] {
        let directory = root.join("mcp").join(category);
        if !exists(&directory)? {
            continue;
        }
        os::validate_tree(root, &directory)?;
        for entry in std::fs::read_dir(&directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(hash) = generation_hash(category, &name) else {
                continue;
            };
            let relative = format!("mcp/{category}/{name}");
            os::validate_tree(root, &entry.path())?;
            let executable = if category == "runtimes" {
                os::CLI_NAME
            } else {
                os::LAUNCHER_NAME
            };
            let image = format!("{relative}/{executable}");
            if exists(&root.join(&image))? {
                let checked = os::open_verified(root, &root.join(&image), &hash)?;
                drop(checked);
                files.push(RemovalFile {
                    path: image,
                    sha256: hash,
                });
            }
            let metadata = if category == "runtimes" {
                "lifetime.lock"
            } else {
                "info.v1.json"
            };
            let metadata = format!("{relative}/{metadata}");
            if exists(&root.join(&metadata))? {
                os::validate_tree(root, &root.join(&metadata))?;
                let bytes = if category == "runtimes" {
                    empty_lock(&root.join(&metadata))?
                } else {
                    std::fs::read(root.join(&metadata)).map_err(|e| e.to_string())?
                };
                if category == "runtimes" {
                    if !bytes.is_empty() {
                        return Err("uninstall_runtime_lock_content_changed".into());
                    }
                } else {
                    let value: serde_json::Value =
                        serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                    if value["product_version"].as_str().is_none_or(str::is_empty)
                        || value
                            != serde_json::json!({"product_version":value["product_version"],"launcher_protocol":1})
                    {
                        return Err("uninstall_launcher_metadata_changed".into());
                    }
                }
                files.push(RemovalFile {
                    path: metadata,
                    sha256: hash_bytes(&bytes),
                });
            }
            directories.push(relative);
        }
        directories.push(format!("mcp/{category}"));
    }
    for entry in std::fs::read_dir(root.join("mcp")).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let relative = format!("mcp/{name}");
        if !removal_top_file(&name) {
            continue;
        }
        os::validate_tree(root, &entry.path())?;
        let bytes = if name.ends_with(".lock") {
            empty_lock(&entry.path())?
        } else {
            std::fs::read(entry.path()).map_err(|e| e.to_string())?
        };
        if name == "installation.id" {
            if bytes != id.as_bytes() {
                return Err("uninstall_installation_id_changed".into());
            }
        } else if name == "install-last-applied.txt" {
            // NSIS's verified bundle helper writes only the bare release
            // version here. Preserve a user-repurposed file at this name.
            let version = std::str::from_utf8(&bytes).ok();
            if !version.is_some_and(|version| {
                version.len() <= 64
                    && version.split('.').count() == 3
                    && version.split('.').all(|part| {
                        !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
                    })
            }) {
                continue;
            }
        } else if name.ends_with(".lock") {
            if !bytes.is_empty() {
                return Err("uninstall_lock_content_changed".into());
            }
        } else if name == "update.stop" {
            if bytes != b"stop" {
                return Err("uninstall_stop_content_changed".into());
            }
        } else if name == "update.v1.json" {
            let state: crate::mcp_launcher::core::lifecycle::UpdateState =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            state.validate(os::CLI_NAME)?;
            if &state.installation_id != id || state.blocks_work() {
                return Err("uninstall_requires_update_recovery".into());
            }
        } else {
            let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            manifest.validate(os::CLI_NAME)?;
            if &manifest.installation_id != id {
                return Err("uninstall_foreign_manifest".into());
            }
        }
        files.push(RemovalFile {
            path: relative,
            sha256: hash_bytes(&bytes),
        });
    }
    Ok(RemovalRecord {
        schema: 1,
        manifest: installation.manifest.clone(),
        files,
        directories,
    })
}

/// The content of a lock file without reading it: a lock file is empty by
/// contract, and on Windows a read of a file whose byte 0 is locked
/// (`os::lock`, held by this removal or by another owner) fails with
/// ERROR_LOCK_VIOLATION (33). Its length is read from the metadata instead;
/// a non-empty lock yields one placeholder byte, which every caller refuses
/// as changed content.
pub(super) fn empty_lock(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    Ok(if metadata.len() == 0 {
        Vec::new()
    } else {
        vec![0]
    })
}

pub(super) fn exists(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

fn generation_hash(category: &str, name: &str) -> Option<String> {
    use crate::mcp_launcher::core::manifest::{is_id, is_sha256};
    if is_sha256(name) {
        return Some(name.into());
    }
    if category == "runtimes" {
        let (hash, id) = name.strip_prefix(".retired-")?.split_once('-')?;
        if is_sha256(hash) && is_id(id) {
            return Some(hash.into());
        }
    }
    None
}

pub(super) fn remaining_managed_files(root: &Path) -> Result<bool, String> {
    let mcp = root.join("mcp");
    os::validate_tree(root, &mcp)?;
    for entry in std::fs::read_dir(&mcp).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name != "install-last-applied.txt" && removal_top_file(name))
        {
            return Ok(true);
        }
    }
    for category in ["runtimes", "launchers"] {
        let directory = mcp.join(category);
        if !exists(&directory)? {
            continue;
        }
        os::validate_tree(root, &directory)?;
        for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry
                .file_name()
                .to_str()
                .is_some_and(|name| generation_hash(category, name).is_some())
            {
                continue;
            }
            os::validate_tree(root, &entry.path())?;
            for name in if category == "runtimes" {
                [os::CLI_NAME, "lifetime.lock"]
            } else {
                [os::LAUNCHER_NAME, "info.v1.json"]
            } {
                if exists(&entry.path().join(name))? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

fn removal_top_file(name: &str) -> bool {
    matches!(
        name,
        "current.v1.json"
            | "legacy.v1.json"
            | "installation.id"
            | "install-last-applied.txt"
            | "update.v1.json"
            | "update.stop"
            | "update-owner.lock"
            | "control.lock"
            | "operations.lock"
    ) || name
        .strip_prefix("retired-current-")
        .and_then(|name| name.strip_suffix(".json"))
        .is_some_and(crate::mcp_launcher::core::manifest::is_id)
}

fn removal_directory_allowed(path: &str) -> bool {
    if matches!(path, "mcp/runtimes" | "mcp/launchers") {
        return true;
    }
    let parts = path.split('/').collect::<Vec<_>>();
    parts.len() == 3
        && parts[0] == "mcp"
        && matches!(parts[1], "runtimes" | "launchers")
        && generation_hash(parts[1], parts[2]).is_some()
}

fn removal_file_allowed(path: &str) -> bool {
    if let Some(name) = path.strip_prefix("mcp/") {
        if removal_top_file(name) {
            return true;
        }
    }
    let parts = path.split('/').collect::<Vec<_>>();
    parts.len() == 4
        && removal_directory_allowed(&parts[..3].join("/"))
        && if parts[1] == "runtimes" {
            matches!(parts[3], "lifetime.lock") || parts[3] == os::CLI_NAME
        } else {
            parts[3] == "info.v1.json" || parts[3] == os::LAUNCHER_NAME
        }
}
