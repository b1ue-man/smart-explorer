//! Data of the surfaces (C4.4, C5.5): the shell is their only data holder.
//! A feed thread reads the Share profiles and, only while the background
//! service already runs, its live state through `share_worker_peek` every
//! 900 ms (the calm GUI cadence `SHARE_IDLE_POLL`). It never starts the
//! service (V11, Vorschlag V1 of C10) and never takes the GUI's events (F8).
//! The button "Verbinden" of a surface starts the service like the se window
//! does, once at a time, in a thread of its own. Drive folders (S2) and
//! vaults (S3) join with DSK-M4.
use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::store::{unix_now, DesktopDir};
use crate::daemon::ShareWorkerSnapshot;
use crate::desktop::core::tiles::{self, ShareState, TileInput, TileModel};
use crate::share::ShareProfiles;

/// Cadence of the feed (C4.4).
pub(crate) const FEED_TICK: Duration = Duration::from_millis(900);
/// Without a service the profiles come from disk every fifth tick.
const PROFILE_RELOAD_TICKS: u64 = 5;
pub(crate) const NO_SERVICE: &str = "Hintergrunddienst läuft nicht";

type PeekFn = dyn Fn() -> Result<Option<ShareWorkerSnapshot>, String> + Send + Sync;
type EnsureFn = dyn Fn() -> Result<(), String> + Send + Sync;

/// Where the feed reads; production uses the daemon client.
#[derive(Clone)]
pub(crate) struct FeedSources {
    pub(crate) peek: Arc<PeekFn>,
    pub(crate) ensure_worker: Arc<EnsureFn>,
}

impl FeedSources {
    pub(crate) fn system() -> FeedSources {
        FeedSources {
            peek: Arc::new(crate::daemon::share_worker_peek),
            ensure_worker: Arc::new(crate::daemon::ensure_worker_ready),
        }
    }
}

/// The latest projection; `rev` grows with every change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FeedState {
    pub(crate) rev: u64,
    pub(crate) tiles: TileModel,
}

pub(crate) struct ModelFeed {
    latest: Arc<Mutex<Option<FeedState>>>,
    stop: Arc<AtomicBool>,
    connecting: Arc<AtomicBool>,
    sources: FeedSources,
    dir: DesktopDir,
}

fn default_home() -> String {
    crate::support_dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .to_string_lossy()
        .replace('\\', "/")
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
        let thread_latest = latest.clone();
        let thread_stop = stop.clone();
        let thread_sources = sources.clone();
        std::thread::Builder::new()
            .name("se-desktop-feed".into())
            .spawn(move || feed_loop(&thread_sources, &thread_latest, &thread_stop))?;
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

fn feed_loop(sources: &FeedSources, latest: &Mutex<Option<FeedState>>, stop: &AtomicBool) {
    let home = default_home();
    let empty_paths = BTreeMap::new();
    let mut stored: Option<Result<ShareProfiles, String>> = None;
    let mut tick: u64 = 0;
    let mut rev: u64 = 0;
    while !stop.load(Ordering::Acquire) {
        let peek = (sources.peek)();
        let share_state = share_state_of(&peek);
        let profiles = match peek {
            Ok(Some(snapshot)) => Ok(snapshot.profiles),
            _ => {
                if stored.is_none() || tick % PROFILE_RELOAD_TICKS == 0 {
                    stored = Some(ShareProfiles::load_checked(Some(home.clone())));
                }
                stored
                    .clone()
                    .unwrap_or_else(|| Err("Share-Profile nicht gelesen".into()))
            }
        };
        let input = TileInput {
            profiles: profiles.as_ref().map_err(String::as_str),
            share_state: &share_state,
            now_secs: unix_now(),
            device_paths: &empty_paths,
            vaults: None,
        };
        let projected = tiles::project(&input);
        if let Ok(mut slot) = latest.lock() {
            if slot.as_ref().map(|state| &state.tiles) != Some(&projected) {
                rev += 1;
                *slot = Some(FeedState {
                    rev,
                    tiles: projected,
                });
            }
        }
        tick += 1;
        let mut waited = Duration::ZERO;
        while waited < FEED_TICK && !stop.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(100));
            waited += Duration::from_millis(100);
        }
    }
}
