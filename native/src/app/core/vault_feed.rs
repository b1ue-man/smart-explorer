//! The feed of the vault windows (U29-2): one background thread
//! `vault-feed` asks the worker for `Vault::Status` every 1000 ms (the
//! rhythm of the watchdog, H5; the cause: the worker has no channel it could
//! push events through, V10), reads the vault list and keeps both in
//! `VaultFeedState`. The interface thread only reads copies. The thread
//! starts with the first frame that draws the section „Tresore“ and ends
//! with the program.
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use super::vault_models::{
    EventModel, RecordModel, SiteModel, StatusModel, VaultFeedState, VaultRow,
};

/// The rhythm of the status request.
const POLL: Duration = Duration::from_millis(1000);

static FEED: OnceLock<Arc<Mutex<VaultFeedState>>> = OnceLock::new();
static WAKE: OnceLock<crossbeam_channel::Sender<()>> = OnceLock::new();
static CONTEXT: OnceLock<eframe::egui::Context> = OnceLock::new();

/// The context of the main window, once the section was drawn: threads that
/// start outside a frame (opening a file) use it to wake the interface.
pub(crate) fn context() -> eframe::egui::Context {
    CONTEXT.get().cloned().unwrap_or_default()
}

/// A lock that survives a panic of another thread (the data are plain values
/// that are valid at every step).
pub(crate) fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn feed() -> &'static Arc<Mutex<VaultFeedState>> {
    FEED.get_or_init(Arc::default)
}

/// Starts the thread once.
pub(crate) fn start(ctx: &eframe::egui::Context) {
    if WAKE.get().is_some() {
        return;
    }
    let _ = CONTEXT.set(ctx.clone());
    let (wake, sleep) = crossbeam_channel::bounded::<()>(1);
    if WAKE.set(wake).is_err() {
        return;
    }
    let ctx = ctx.clone();
    let state = feed().clone();
    let spawned = std::thread::Builder::new()
        .name("vault-feed".into())
        .spawn(move || loop {
            let status = crate::daemon::vault_status()
                .map(|status| status.as_ref().map(status_model))
                .map_err(|error| error.message);
            let records = crate::vault::RegistryStore::open_default()
                .load()
                .map(|load| load.sites.iter().map(record_model).collect())
                .map_err(|error| format!("Tresorliste lesen: {error}"));
            let changed = guard(&state).apply(status, records);
            if changed {
                ctx.request_repaint();
            }
            // A poke wakes the thread early; the sender lives as long as the
            // program, so the wait only ever times out or is woken.
            let _ = sleep.recv_timeout(POLL);
        });
    if let Err(error) = spawned {
        guard(feed()).error = Some(format!("Tresor-Anzeige konnte nicht starten: {error}"));
    }
}

/// Asks the thread to look now (after an unlock, a lock, a change of the
/// list).
pub(crate) fn poke() {
    if let Some(wake) = WAKE.get() {
        let _ = wake.try_send(());
    }
}

/// The rows, the status kept and the sites that were locked since the last
/// call (their tabs close).
pub(crate) fn snapshot() -> Snapshot {
    let mut state = guard(feed());
    let rows = state.rows();
    let locked = state.newly_locked(&rows);
    Snapshot {
        rows,
        error: state.error.clone(),
        locked,
    }
}

/// What one frame reads of the feed.
pub(crate) struct Snapshot {
    pub(crate) rows: Vec<VaultRow>,
    pub(crate) error: Option<String>,
    pub(crate) locked: Vec<String>,
}

/// What a background job of a window has delivered.
pub(crate) enum Poll<T> {
    Pending,
    Ready(T),
    /// The thread ended without an answer.
    Gone,
}

/// Runs `work` on a named thread and wakes the interface with its result.
pub(crate) fn spawn<T: Send + 'static>(
    ctx: &eframe::egui::Context,
    name: &str,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<crossbeam_channel::Receiver<T>, String> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let value = work();
            let _ = tx.send(value);
            ctx.request_repaint();
        })
        .map(|_| rx)
        .map_err(|error| format!("Hintergrundarbeit konnte nicht starten: {error}"))
}

/// Looks at the answer of a job; a delivered or lost answer empties `slot`.
pub(crate) fn poll<T>(slot: &mut Option<crossbeam_channel::Receiver<T>>) -> Poll<T> {
    let Some(rx) = slot.as_ref() else {
        return Poll::Pending;
    };
    match rx.try_recv() {
        Ok(value) => {
            *slot = None;
            Poll::Ready(value)
        }
        Err(crossbeam_channel::TryRecvError::Empty) => Poll::Pending,
        Err(crossbeam_channel::TryRecvError::Disconnected) => {
            *slot = None;
            Poll::Gone
        }
    }
}

/// The last status of the worker (`None`: no worker runs or none was read).
pub(crate) fn status() -> Option<StatusModel> {
    guard(feed()).status.clone()
}

/// The rows without evaluating the lock events (windows that only list).
pub(crate) fn rows() -> Vec<VaultRow> {
    guard(feed()).rows()
}

/// Sets what the feed holds (the render test of the harness; no thread, no
/// request to the service).
#[cfg(test)]
pub(crate) fn set_for_test(status: Option<StatusModel>, records: Vec<RecordModel>) {
    guard(feed()).apply(Ok(status), Ok(records));
}

/// The status model of the wire view.
fn status_model(view: &crate::daemon::VaultStatusView) -> StatusModel {
    StatusModel {
        sites: view
            .sites
            .iter()
            .map(|site| SiteModel {
                site: site.site.clone(),
                label: site.label.clone(),
                state: site.state.clone(),
                reason: site.reason.clone(),
                remember_password: site.remember_password,
                shortening_threshold: site.shortening_threshold,
            })
            .collect(),
        event_seq: view.event_seq,
        events: view
            .events
            .iter()
            .map(|event| EventModel {
                seq: event.seq,
                site: event.site.clone(),
                state: event.state.clone(),
                reason: event.reason.clone(),
            })
            .collect(),
        sleep_lock: view.sleep_lock,
        screen_lock: view.screen_lock,
        screen_lock_note: view.screen_lock_note.clone(),
    }
}

fn record_model(site: &crate::vault::SiteRecord) -> RecordModel {
    RecordModel {
        id: site.id.clone(),
        label: site.label.clone(),
        location: site.location.clone(),
        vault_id: site.vault_id.clone(),
        replica_of: site.replica_of.clone(),
        remember_password: site.remember_password,
        auto_lock_idle_min: site.lock.auto_lock_idle_min,
        lock_on_sleep: site.lock.lock_on_sleep,
        lock_on_screen_lock: site.lock.lock_on_screen_lock,
    }
}
