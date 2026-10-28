//! The background service's device feed for the device pages (design 5.2,
//! user rule r5): windows, services and the Remote Desktop state of each
//! present device arrive without any click. While a page shows a device,
//! its live state is read through `share_worker_peek` (never the window's
//! own events, never a service start) at most once per display tick, the
//! cadence se Desktop uses for the same state (`model_feed::FEED_TICK`).
use crate::daemon::ShareWorkerSnapshot;
use crate::desktop::core::tiles::{self, TileFeed};
use eframe::egui;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Display tick of the feed pages; a rate, not a time limit.
const TICK: Duration = Duration::from_millis(900);
/// Shown while the background service does not run.
pub(crate) const NO_SERVICE: &str =
    "Hintergrunddienst läuft nicht; Fenster, Dienste und Remote Desktop erscheinen, sobald er läuft.";

struct Cache {
    snapshot: Option<ShareWorkerSnapshot>,
    error: Option<String>,
    read_at: Option<Instant>,
    reading: bool,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    snapshot: None,
    error: None,
    read_at: None,
    reading: false,
});

/// What a page shows for one device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FeedRead<T> {
    /// The first read has not finished yet.
    Pending,
    /// The background service is unreachable or does not run.
    Unavailable(String),
    Ready(T),
}

/// Reads `read` from the latest live state and keeps it current while the
/// page is drawn.
pub(crate) fn read<T>(
    ctx: &egui::Context,
    read: impl FnOnce(&ShareWorkerSnapshot) -> T,
) -> FeedRead<T> {
    refresh(ctx);
    let cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    match (&cache.snapshot, &cache.error) {
        (Some(snapshot), _) => FeedRead::Ready(read(snapshot)),
        (None, Some(error)) => FeedRead::Unavailable(error.clone()),
        (None, None) => FeedRead::Pending,
    }
}

/// The projected feed of one device (`None` while the service holds none).
pub(crate) fn device(ctx: &egui::Context, device: &str, key: &str) -> FeedRead<Option<TileFeed>> {
    read(ctx, |snapshot| tiles::feed_of(snapshot, device, key))
}

fn refresh(ctx: &egui::Context) {
    ctx.request_repaint_after(TICK);
    {
        let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.reading || cache.read_at.is_some_and(|at| at.elapsed() < TICK) {
            return;
        }
        cache.reading = true;
    }
    let context = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("device-feed".into())
        .spawn(move || {
            let result = crate::daemon::share_worker_peek();
            let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
            cache.reading = false;
            cache.read_at = Some(Instant::now());
            match result {
                Ok(Some(snapshot)) => {
                    cache.snapshot = Some(snapshot);
                    cache.error = None;
                }
                Ok(None) => {
                    cache.snapshot = None;
                    cache.error = Some(NO_SERVICE.into());
                }
                Err(error) => {
                    cache.snapshot = None;
                    cache.error = Some(format!("Hintergrunddienst nicht erreichbar: {error}"));
                }
            }
            drop(cache);
            context.request_repaint();
        });
    if let Err(error) = spawned {
        let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        cache.reading = false;
        cache.read_at = Some(Instant::now());
        cache.error = Some(format!("Gerätefeed nicht gelesen: {error}"));
    }
}
