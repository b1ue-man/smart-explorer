//! Private, bounded geometry metadata, separate from all egui widget memory.
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

#[derive(Default, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct GeometryFile {
    version: u32,
    sizes: BTreeMap<String, [f32; 2]>,
}

struct Store {
    file: GeometryFile,
    attempted: BTreeMap<String, [f32; 2]>,
    writable: bool,
    failed: bool,
}

fn valid(size: [f32; 2]) -> bool {
    size.into_iter().all(|value| value.is_finite() && value > 0.0 && value < 100_000.0)
}

fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(|| {
        let path = crate::support_dirs::app_data_file("dialog_geometry.json");
        let loaded = crate::support_dirs::read_private_text(&path, 64 * 1024);
        let (mut file, writable) = match loaded {
            Ok(text) => match serde_json::from_str::<GeometryFile>(&text) {
                Ok(file) if file.version == 1 => (file, true),
                // Preserve unrecognized/corrupt contents, including user files.
                _ => (GeometryFile::default(), false),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (
                GeometryFile { version: 1, ..GeometryFile::default() }, true,
            ),
            Err(_) => (GeometryFile::default(), false),
        };
        file.sizes.retain(|key, size| KEYS.contains(&key.as_str()) && valid(*size));
        Mutex::new(Store { file, attempted: BTreeMap::new(), writable, failed: !writable })
    })
}

pub(in crate::app) fn load(key: &'static str) -> Option<[f32; 2]> {
    store().lock().ok()?.file.sizes.get(key).copied()
}

pub(in crate::app) fn failed() -> bool {
    store().lock().map(|state| state.failed).unwrap_or(true)
}

/// Called after the pointer is released. A failed identical write is not
/// retried every frame; the UI reports the persistence limit explicitly.
pub(in crate::app) fn save(key: &'static str, size: [f32; 2]) {
    if !KEYS.contains(&key) || !valid(size) { return; }
    let Ok(mut state) = store().lock() else { return };
    if !state.writable { return; }
    let previous = state.attempted.get(key).or_else(|| state.file.sizes.get(key));
    if previous.is_some_and(|old| {
        (old[0] - size[0]).abs() < 0.5 && (old[1] - size[1]).abs() < 0.5
    }) { return; }
    state.attempted.insert(key.to_owned(), size);
    let result = merge_save(key, size);
    state.failed = result.is_err();
    if let Ok(file) = result { state.file = file; }
}

/// Serialize read/merge/promotion across GUI processes. Atomic replacement
/// alone cannot preserve a second process's independently changed dialog.
fn merge_save(key: &str, size: [f32; 2]) -> std::io::Result<GeometryFile> {
    let path = crate::support_dirs::app_data_file("dialog_geometry.json");
    let lock = crate::support_dirs::open_private_lock(
        &crate::support_dirs::app_data_file("dialog_geometry.lock"),
    )?;
    lock.lock()?;
    let mut file = match crate::support_dirs::read_private_text(&path, 64 * 1024) {
        Ok(text) => serde_json::from_str::<GeometryFile>(&text).map_err(std::io::Error::other)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound =>
            GeometryFile { version: 1, ..GeometryFile::default() },
        Err(error) => return Err(error),
    };
    if file.version != 1 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "unknown dialog geometry version"));
    }
    file.sizes.retain(|key, size| KEYS.contains(&key.as_str()) && valid(*size));
    file.sizes.insert(key.to_owned(), size);
    let bytes = serde_json::to_vec(&file).map_err(std::io::Error::other)?;
    crate::support_dirs::write_private_atomic(&path, &bytes)?;
    Ok(file)
}

// Closed set of dialog kinds. Never extend this with a path or a dynamic title.
const KEYS: &[&str] = &[
    "analytics_ui.0",
    "bisync_conflict_ui.0",
    "bisync_ui.0",
    "connect_dialog_ui.0",
    "copy_dialog.0",
    "dialogs.0",
    "dialogs.1",
    "dialogs.2",
    "dialogs.3",
    "error_log_ui.0",
    "host_key_ui.0",
    "job_editor_ui.0",
    "licenses_ui.0",
    "menus_sync_jobs.0",
    "menus_sync_jobs.1",
    "merge_ui.0",
    "mount_ui.0",
    "mount_ui_draft.0",
    "picker.analytics",
    "picker.bisync",
    "picker.copy",
    "picker.mirror",
    "picker.open",
    "picker.reclaim",
    "picker.save",
    "picker.source",
    "picker.target",
    "reclaim_ui.0",
    "settings_ui.0",
    "share_access_dialog_ui.0",
    "share_access_room_ui.0",
    "share_screen_codec_ui.0",
    "share_screen_codec_ui.1",
    "share_window_ui.0",
    "sync_exit_gate.0",
    "sync_job_log_ui.0",
    "sync_job_state_ui.0",
    "sync_versions_ui.0",
    "transfer_window.0",
    "vault_actions_ui.0",
    "vault_add_ui.0",
    "vault_check_ui.0",
    "vault_compare_ui.0",
    "vault_compare_ui.1",
    "vault_create_ui.0",
    "vault_settings_ui.0",
    "vault_sync_editor_ui.0",
    "vault_unlock_ui.0",
    "vault_unlock_ui.1",
];
