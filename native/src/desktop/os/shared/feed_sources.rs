//! Sources of the shell's model besides the Share profiles (DSK-M4, C4.4):
//! the S2 device tree (`Drive::DevicesStatus` once, then the long poll
//! `DevicesWatch { since }`, V4), the S3 vault sites (`Vault::Status` of
//! VLT-M4 in the feed's 900-ms tick, V10, H5) and the desktop folders (top
//! level watched through `crate::watch`, read again after a notice). Every
//! daemon read goes through the reads without service start
//! (`request_if_running`, V11).
//!
//! Annahme until DRV-M6 (U17) delivers its variants: the device tree is the
//! V4 object, as the answer itself or as one field of it. A worker without
//! the variants refuses the read; the device tree then counts as unknown,
//! and nothing fails. Device keys are compared with `%XX` in capitals, as
//! `tiles::device_key` writes them.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::desktop::core::items::DesktopFile;
use crate::desktop::core::tiles::VaultTile;

/// Suite switch (DSK-M15): the user's desktop folder of a test.
pub(crate) const TEST_DESKTOP_DIR: &str = "SE_DESKTOP_TEST_DESKTOP_DIR";
/// After a refused or failed device-tree read the next one waits this long.
const RETRY_AFTER: Duration = Duration::from_secs(30);
/// Least pause between two answers of the long poll.
const WATCH_PAUSE: Duration = Duration::from_millis(200);
const WAIT_SLICE: Duration = Duration::from_millis(100);
/// Entries of one desktop folder the model carries at most.
const MAX_FOLDER_ENTRIES: usize = 2_000;
/// Names the desktops never show (Windows), compared without case.
const ALWAYS_HIDDEN: [&str; 2] = ["desktop.ini", "thumbs.db"];
const MAX_SITE_ID: usize = 128;

/// One read of the daemon without service start: `Ok(None)` without worker.
pub(crate) type JsonRead = dyn Fn() -> Result<Option<Value>, String> + Send + Sync;
/// The long poll `DevicesWatch { since }`.
pub(crate) type WatchRead = dyn Fn(u64) -> Result<Option<Value>, String> + Send + Sync;
/// The vault sites of a running worker as tiles.
pub(crate) type VaultRead = dyn Fn() -> Result<Option<Vec<VaultTile>>, String> + Send + Sync;

/// The S2 device tree as the shell knows it (V4).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DeviceTreeStatus {
    pub(crate) enabled: bool,
    pub(crate) state: String,
    pub(crate) mounted: bool,
    pub(crate) root_path: Option<String>,
    pub(crate) revision: u64,
    /// Device key (`%XX` in capitals) → folder of the device.
    pub(crate) paths: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum DeviceTreeView {
    /// No worker of this version runs, or none answered yet.
    #[default]
    NotRunning,
    /// The worker refused or failed the read (before DRV-M6: unknown request).
    Unknown(String),
    Known(DeviceTreeStatus),
}

impl DeviceTreeView {
    /// Folders of the devices while the device tree is mounted.
    pub(crate) fn mounted_paths(&self) -> Option<&BTreeMap<String, String>> {
        match self {
            DeviceTreeView::Known(status) if status.mounted => Some(&status.paths),
            _ => None,
        }
    }

    pub(crate) fn mounted(&self) -> bool {
        self.mounted_paths().is_some()
    }

    /// `root_path` of a mounted device tree (`devices_folder` of the status).
    pub(crate) fn root_path(&self) -> Option<&str> {
        match self {
            DeviceTreeView::Known(status) if status.mounted => status.root_path.as_deref(),
            _ => None,
        }
    }
}

/// `%xx` escapes in capitals, as `tiles::device_key` writes them.
pub(crate) fn normalize_key(key: &str) -> String {
    let mut normalized = String::with_capacity(key.len());
    let mut escaped = 0u8;
    for ch in key.chars() {
        if escaped > 0 {
            normalized.push(ch.to_ascii_uppercase());
            escaped -= 1;
            continue;
        }
        if ch == '%' {
            escaped = 2;
        }
        normalized.push(ch);
    }
    normalized
}

/// The V4 object of an answer: the answer itself or one of its fields.
fn device_tree_object(value: &Value) -> Option<&Map<String, Value>> {
    let object = value.as_object()?;
    let has_devices =
        |object: &Map<String, Value>| object.get("devices").is_some_and(Value::is_array);
    if has_devices(object) {
        return Some(object);
    }
    object
        .values()
        .filter_map(Value::as_object)
        .find(|inner| has_devices(inner))
}

/// The device tree of a `DevicesStatus` or `DevicesWatch` answer.
pub(crate) fn parse_device_tree(value: &Value) -> Option<DeviceTreeStatus> {
    let object = device_tree_object(value)?;
    let text = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_string);
    let flag = |key: &str| object.get(key).and_then(Value::as_bool).unwrap_or(false);
    let mut paths = BTreeMap::new();
    for device in object
        .get("devices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = device.get("key").and_then(Value::as_str);
        let path = device
            .get("path")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty());
        if let (Some(key), Some(path)) = (key, path) {
            paths.insert(normalize_key(key), path.to_string());
        }
    }
    Some(DeviceTreeStatus {
        enabled: flag("enabled"),
        state: text("state").unwrap_or_default(),
        mounted: flag("mounted"),
        root_path: text("root_path").filter(|path| !path.is_empty()),
        revision: object.get("revision").and_then(Value::as_u64).unwrap_or(0),
        paths,
    })
}

/// The view of one device-tree read.
pub(crate) fn device_tree_view(read: Result<Option<Value>, String>) -> DeviceTreeView {
    match read {
        Ok(Some(answer)) => match parse_device_tree(&answer) {
            Some(status) => DeviceTreeView::Known(status),
            None => DeviceTreeView::Unknown("Gerätebaum: unerwartete Antwort".into()),
        },
        Ok(None) => DeviceTreeView::NotRunning,
        Err(error) => DeviceTreeView::Unknown(error),
    }
}

/// One read of the device tree now (`se desktop status`, `se desktop config`
/// and the settings: `mounted`, `root_path`).
pub(crate) fn device_tree_now() -> DeviceTreeView {
    device_tree_view(crate::daemon::drive_devices_peek())
}

/// The vault sites of a running worker (`Vault::Status`, S5-V1) as tiles.
pub(crate) fn vault_sites_now() -> Result<Option<Vec<VaultTile>>, String> {
    let status = crate::daemon::vault_status_peek()?;
    Ok(status.map(|status| {
        vault_tiles(
            status
                .sites
                .into_iter()
                .map(|site| (site.site, site.label, site.state)),
        )
    }))
}

/// Tiles of `(site, label, state)` in their order, each site once: only ID,
/// label and state (S5-V5); an empty label shows the ID; an ID that cannot
/// be a locator is left out.
pub(crate) fn vault_tiles(
    sites: impl IntoIterator<Item = (String, String, String)>,
) -> Vec<VaultTile> {
    let mut tiles: Vec<VaultTile> = Vec::new();
    for (site, label, state) in sites {
        if !valid_site(&site) || tiles.iter().any(|known| known.site == site) {
            continue;
        }
        let label = match label.trim() {
            "" => site.clone(),
            trimmed => trimmed.to_string(),
        };
        tiles.push(VaultTile { site, label, state });
    }
    tiles
}

/// Site IDs go into `vault://<site>/` and `vault:<site>`: 1 to 128 of
/// `A-Z a-z 0-9 . _ -`, not `.` or `..`.
fn valid_site(site: &str) -> bool {
    let allowed = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-');
    !site.is_empty()
        && site.len() <= MAX_SITE_ID
        && site != "."
        && site != ".."
        && site.bytes().all(allowed)
}

/// Waits `pause` in slices, ending early when `stop` is set.
pub(crate) fn wait(pause: Duration, stop: &AtomicBool) {
    let mut waited = Duration::ZERO;
    while waited < pause && !stop.load(Ordering::Acquire) {
        std::thread::sleep(WAIT_SLICE.min(pause - waited));
        waited += WAIT_SLICE;
    }
}

/// The device tree kept current by a thread: `DevicesStatus` once, then
/// `DevicesWatch { since }`, which answers after a change or after 30 s.
pub(crate) struct DeviceTreeFeed {
    latest: Arc<Mutex<DeviceTreeView>>,
    stop: Arc<AtomicBool>,
}

impl DeviceTreeFeed {
    pub(crate) fn start(status: Arc<JsonRead>, watch: Arc<WatchRead>) -> std::io::Result<Self> {
        let latest = Arc::new(Mutex::new(DeviceTreeView::NotRunning));
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_latest, thread_stop) = (latest.clone(), stop.clone());
        std::thread::Builder::new()
            .name("se-desktop-devices".into())
            .spawn(move || device_loop(&status, &watch, &thread_latest, &thread_stop))?;
        Ok(DeviceTreeFeed { latest, stop })
    }

    pub(crate) fn latest(&self) -> DeviceTreeView {
        self.latest
            .lock()
            .map(|view| view.clone())
            .unwrap_or_default()
    }
}

impl Drop for DeviceTreeFeed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn device_loop(
    status: &JsonRead,
    watch: &WatchRead,
    latest: &Mutex<DeviceTreeView>,
    stop: &AtomicBool,
) {
    let mut since: Option<u64> = None;
    while !stop.load(Ordering::Acquire) {
        let read = match since {
            Some(revision) => watch(revision),
            None => status(),
        };
        let view = device_tree_view(read);
        let pause = match &view {
            DeviceTreeView::Known(tree) => {
                since = Some(tree.revision);
                WATCH_PAUSE
            }
            DeviceTreeView::NotRunning => {
                since = None;
                super::model_feed::FEED_TICK
            }
            DeviceTreeView::Unknown(_) => {
                since = None;
                RETRY_AFTER
            }
        };
        if let Ok(mut slot) = latest.lock() {
            *slot = view;
        }
        wait(pause, stop);
    }
}

/// A desktop folder of this user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DesktopFolder {
    pub(crate) path: PathBuf,
    /// The all-users desktop (Windows `FOLDERID_PublicDesktop`).
    pub(crate) public: bool,
    /// `.desktop` files are starters (Linux, C2.7).
    pub(crate) launchers: bool,
}

/// The desktop folders of this user. DSK-M5 (Windows `FOLDERID_Desktop`,
/// `FOLDERID_PublicDesktop`) and DSK-M6 (Linux `XDG_DESKTOP_DIR`) return the
/// platform's folders here; until then only the suite switch names one.
pub(crate) fn desktop_folders() -> Vec<DesktopFolder> {
    std::env::var_os(TEST_DESKTOP_DIR)
        .filter(|value| !value.is_empty())
        .map(|value| {
            vec![DesktopFolder {
                path: PathBuf::from(value),
                public: false,
                launchers: false,
            }]
        })
        .unwrap_or_default()
}

/// The entries of the desktop folders as the model carries them.
pub(crate) fn scan_folders(folders: &[DesktopFolder]) -> Vec<DesktopFile> {
    let mut files = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(&folder.path) else {
            continue;
        };
        for entry in entries.flatten().take(MAX_FOLDER_ENTRIES) {
            // A name that is no Unicode cannot be shown or addressed by name.
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let is_dir = std::fs::metadata(entry.path()).is_ok_and(|meta| meta.is_dir());
            let lower = name.to_lowercase();
            files.push(DesktopFile {
                hidden: name.starts_with('.') || ALWAYS_HIDDEN.contains(&lower.as_str()),
                launcher: folder.launchers && !is_dir && lower.ends_with(".desktop"),
                public: folder.public,
                is_dir,
                name,
            });
        }
    }
    files
}

/// Change notices of the desktop folders, top level only (links below are
/// never followed by the watch service).
pub(crate) struct FolderWatch {
    folders: Vec<DesktopFolder>,
    _handles: Vec<crate::watch::WatchHandle>,
    changed: Arc<AtomicBool>,
}

impl FolderWatch {
    pub(crate) fn new(folders: Vec<DesktopFolder>) -> FolderWatch {
        let changed = Arc::new(AtomicBool::new(true));
        let handles = folders
            .iter()
            .filter_map(|folder| {
                let flag = changed.clone();
                let filter = crate::watch::WatchFilter::new(|entry| !entry.rel.contains('/'));
                let sink = crate::watch::WatchSink::callback(move |_| {
                    flag.store(true, Ordering::Release);
                });
                let options = crate::watch::WatchOptions::default();
                crate::watch::watch(&folder.path, options, filter, sink).ok()
            })
            .collect();
        FolderWatch {
            folders,
            _handles: handles,
            changed,
        }
    }

    pub(crate) fn folders(&self) -> &[DesktopFolder] {
        &self.folders
    }

    /// Whether a notice came since the last call (the first call: yes).
    pub(crate) fn take_change(&self) -> bool {
        self.changed.swap(false, Ordering::AcqRel)
    }
}

#[cfg(test)]
#[path = "feed_task_tests.rs"]
mod task_tests;
#[cfg(test)]
#[path = "feed_sources_tests.rs"]
mod tests;
