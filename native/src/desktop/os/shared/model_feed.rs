//! Data of the surfaces (C4.4, C5.5): the shell is their only data holder.
//! A feed thread reads the Share profiles and, only while the background
//! service already runs, its live state through `share_worker_peek` every
//! 900 ms (the calm GUI cadence `SHARE_IDLE_POLL`), in the same tick the S3
//! vault sites (`Vault::Status`, H5), and the desktop folders after a change
//! notice; a thread of `feed_sources` follows the S2 device tree with its long
//! poll (`DevicesWatch`). Nothing here starts the service (V11, Vorschlag V1
//! of C10) or takes the GUI's events (F8). The button "Verbinden" of a
//! surface starts the service like the se window does, once at a time, in a
//! thread of its own.
use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::feed_sources::{
    self, DesktopFolder, DeviceTreeFeed, DeviceTreeView, FolderWatch, JsonRead, VaultRead,
    WatchRead,
};
use super::store::{unix_now, DesktopDir};
use crate::daemon::ShareWorkerSnapshot;
use crate::desktop::core::items::DesktopFile;
use crate::desktop::core::tiles::{self, ShareState, TileInput, TileModel, VaultTile};
use crate::share::ShareProfiles;

/// Cadence of the feed (C4.4).
pub(crate) const FEED_TICK: Duration = Duration::from_millis(900);
/// Without a service the profiles come from disk every fifth tick.
const PROFILE_RELOAD_TICKS: u64 = 5;
/// The desktop folders are read again after a notice, and every tenth tick
/// in any case (a folder may be unwatchable).
const FILE_RESCAN_TICKS: u64 = 10;
/// The list of desktop folders is asked again about every 30 s.
const FOLDER_CHECK_TICKS: u64 = 33;
/// After a refused vault read the next one waits about 30 s.
const VAULT_RETRY_TICKS: u64 = 33;
pub(crate) const NO_SERVICE: &str = "Hintergrunddienst läuft nicht";

type PeekFn = dyn Fn() -> Result<Option<ShareWorkerSnapshot>, String> + Send + Sync;
type EnsureFn = dyn Fn() -> Result<(), String> + Send + Sync;
type FoldersFn = dyn Fn() -> Vec<DesktopFolder> + Send + Sync;

/// Where the feed reads; production uses the daemon's reads without start.
#[derive(Clone)]
pub(crate) struct FeedSources {
    pub(crate) peek: Arc<PeekFn>,
    pub(crate) ensure_worker: Arc<EnsureFn>,
    /// S2 `DevicesStatus` (V4).
    pub(crate) devices: Arc<JsonRead>,
    /// S2 `DevicesWatch { since }` (V4).
    pub(crate) devices_watch: Arc<WatchRead>,
    /// S3 `Vault::Status` (V10) as tiles.
    pub(crate) vaults: Arc<VaultRead>,
    /// The desktop folders (DSK-M5, DSK-M6).
    pub(crate) desktop_folders: Arc<FoldersFn>,
}

impl FeedSources {
    pub(crate) fn system() -> FeedSources {
        FeedSources {
            peek: Arc::new(crate::daemon::share_worker_peek),
            ensure_worker: Arc::new(crate::daemon::ensure_worker_ready),
            devices: Arc::new(crate::daemon::drive_devices_peek),
            devices_watch: Arc::new(crate::daemon::drive_devices_watch_peek),
            vaults: Arc::new(feed_sources::vault_sites_now),
            desktop_folders: Arc::new(feed_sources::desktop_folders),
        }
    }
}

/// The latest projection; `rev` grows with every change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FeedState {
    pub(crate) rev: u64,
    pub(crate) tiles: TileModel,
    /// Entries of the desktop folders (C1.2, C2.7).
    pub(crate) files: Vec<DesktopFile>,
    /// The S2 device tree; `mounted` decides "Geräte öffnen mit: Dateimanager".
    pub(crate) device_tree: DeviceTreeView,
}

pub(crate) struct ModelFeed {
    latest: Arc<Mutex<Option<FeedState>>>,
    stop: Arc<AtomicBool>,
    connecting: Arc<AtomicBool>,
    sources: FeedSources,
    dir: DesktopDir,
}

fn share_state_of(peek: &Result<Option<ShareWorkerSnapshot>, String>) -> ShareState {
    match peek {
        Ok(Some(_)) => ShareState::Live,
        Ok(None) => ShareState::Profiles {
            reason: NO_SERVICE.into(),
        },
        Err(error) => ShareState::Profiles {
            reason: error.clone(),
        },
    }
}

impl ModelFeed {
    pub(crate) fn start(sources: FeedSources, dir: DesktopDir) -> io::Result<ModelFeed> {
        let latest = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let devices =
            DeviceTreeFeed::start(sources.devices.clone(), sources.devices_watch.clone())?;
        let (thread_latest, thread_stop) = (latest.clone(), stop.clone());
        let mut round = FeedRound::new(sources.clone());
        std::thread::Builder::new()
            .name("se-desktop-feed".into())
            .spawn(move || {
                while !thread_stop.load(Ordering::Acquire) {
                    if let Some(state) = round.step(&devices.latest()) {
                        if let Ok(mut slot) = thread_latest.lock() {
                            *slot = Some(state);
                        }
                    }
                    feed_sources::wait(FEED_TICK, &thread_stop);
                }
                drop(devices);
            })?;
        Ok(ModelFeed {
            latest,
            stop,
            connecting: Arc::new(AtomicBool::new(false)),
            sources,
            dir,
        })
    }

    pub(crate) fn latest(&self) -> Option<FeedState> {
        self.latest.lock().ok().and_then(|latest| latest.clone())
    }

    /// "Verbinden": starts the background service in a thread; a second
    /// press while the first runs does nothing.
    pub(crate) fn connect(&self) {
        if self.connecting.swap(true, Ordering::AcqRel) {
            return;
        }
        let connecting = self.connecting.clone();
        let ensure = self.sources.ensure_worker.clone();
        let dir = self.dir.clone();
        let spawned = std::thread::Builder::new()
            .name("se-desktop-connect".into())
            .spawn(move || {
                match ensure() {
                    Ok(()) => dir.log("connect", "Hintergrunddienst bereit"),
                    Err(error) => dir.log("connect_failed", &error),
                }
                connecting.store(false, Ordering::Release);
            });
        if spawned.is_err() {
            self.connecting.store(false, Ordering::Release);
        }
    }
}

impl Drop for ModelFeed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// One projection per tick, apart from the thread (the tests drive it).
pub(crate) struct FeedRound {
    sources: FeedSources,
    home: String,
    stored: Option<Result<ShareProfiles, String>>,
    tick: u64,
    rev: u64,
    vault_retry_at: u64,
    folders: Option<FolderWatch>,
    files: Vec<DesktopFile>,
    last: Option<FeedState>,
}

impl FeedRound {
    pub(crate) fn new(sources: FeedSources) -> FeedRound {
        let home = crate::support_dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .to_string_lossy()
            .replace('\\', "/");
        FeedRound {
            sources,
            home,
            stored: None,
            tick: 0,
            rev: 0,
            vault_retry_at: 0,
            folders: None,
            files: Vec::new(),
            last: None,
        }
    }

    /// The state of this tick; `Some` only when it differs from the last.
    pub(crate) fn step(&mut self, device_tree: &DeviceTreeView) -> Option<FeedState> {
        let peek = (self.sources.peek)();
        let share_state = share_state_of(&peek);
        let running = matches!(peek, Ok(Some(_)));
        let profiles = match peek {
            Ok(Some(snapshot)) => Ok(snapshot.profiles),
            _ => self.stored_profiles(),
        };
        let vaults = self.vaults(running);
        self.refresh_files();
        let no_paths = BTreeMap::new();
        let input = TileInput {
            profiles: profiles.as_ref().map_err(String::as_str),
            share_state: &share_state,
            now_secs: unix_now(),
            device_paths: device_tree.mounted_paths().unwrap_or(&no_paths),
            vaults: vaults.as_deref(),
        };
        let tiles = tiles::project(&input);
        self.tick += 1;
        let unchanged = self.last.as_ref().is_some_and(|last| {
            last.tiles == tiles && last.files == self.files && last.device_tree == *device_tree
        });
        if unchanged {
            return None;
        }
        self.rev += 1;
        let state = FeedState {
            rev: self.rev,
            tiles,
            files: self.files.clone(),
            device_tree: device_tree.clone(),
        };
        self.last = Some(state.clone());
        Some(state)
    }

    fn stored_profiles(&mut self) -> Result<ShareProfiles, String> {
        if self.stored.is_none() || self.tick.is_multiple_of(PROFILE_RELOAD_TICKS) {
            self.stored = Some(ShareProfiles::load_checked(Some(self.home.clone())));
        }
        self.stored
            .clone()
            .unwrap_or_else(|| Err("Share-Profile nicht gelesen".into()))
    }

    /// The vault sites while the service runs; `None` without service and
    /// while a refused read waits for its retry ("Tresore: …", C4.4).
    fn vaults(&mut self, running: bool) -> Option<Vec<VaultTile>> {
        if !running || self.tick < self.vault_retry_at {
            return None;
        }
        match (self.sources.vaults)() {
            Ok(Some(sites)) => Some(sites),
            Ok(None) => None,
            Err(_) => {
                self.vault_retry_at = self.tick + VAULT_RETRY_TICKS;
                None
            }
        }
    }

    fn refresh_files(&mut self) {
        if self.folders.is_none() || self.tick.is_multiple_of(FOLDER_CHECK_TICKS) {
            let folders = (self.sources.desktop_folders)();
            let same = self
                .folders
                .as_ref()
                .is_some_and(|watch| watch.folders() == folders.as_slice());
            if !same {
                self.folders = Some(FolderWatch::new(folders));
            }
        }
        let Some(watch) = &self.folders else {
            return;
        };
        if watch.take_change() || self.tick.is_multiple_of(FILE_RESCAN_TICKS) {
            self.files = feed_sources::scan_folders(watch.folders());
        }
    }
}
