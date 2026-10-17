//! Linux lock signals over the system D-Bus (C3.4; logind, also elogind):
//!
//! - Sleep: the signal `PrepareForSleep(true)` of the manager object locks
//!   the sites that lock on sleep. While such a site is unlocked the worker
//!   holds a `delay` inhibitor (`Inhibit("sleep", …, "delay")`, as
//!   `keep_awake/os/linux_os.rs`), so the lock finishes before the system
//!   sleeps; the inhibitor is released right after and taken again on
//!   `PrepareForSleep(false)`.
//! - Screen lock: on the concrete session object (the `Id` of
//!   `/org/freedesktop/login1/session/auto`, then `GetSession`; signals never
//!   arrive at `session/auto`) a change of `LockedHint` to true and the
//!   signal `Lock`. Without a login session (a worker without a graphical
//!   session) the switch is not available. Unverified assumption A2: GNOME
//!   Shell and KDE Plasma set `LockedHint`.
//!
//! Every D-Bus call runs on threads of their own, never in the caller's
//! thread (zbus's blocking calls may not run inside an async runtime).
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use zbus::blocking::{Connection, MessageIterator};
use zbus::message::{Message, Type};
use zbus::zvariant::{ObjectPath, OwnedFd, OwnedObjectPath, OwnedValue};
use zbus::MatchRule;

use crate::vault::core::lock_policy::{SignalControl, SignalSink, SignalSupport, SystemSignal};

const LOGIN1: &str = "org.freedesktop.login1";
const LOGIN1_PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION: &str = "org.freedesktop.login1.Session";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const AUTO_SESSION: &str = "/org/freedesktop/login1/session/auto";

/// What surfaces say next to „Bei Bildschirmsperre sperren“ (C2 S3 K13).
pub(super) const SCREEN_LOCK_NOTE: Option<&str> =
    Some("Unter Linux nur, wenn die Desktopumgebung die Sitzungssperre meldet.");

#[derive(Default)]
struct Shared {
    /// Subscribed to the signals of logind.
    ready: AtomicBool,
    sleep: AtomicBool,
    screen_lock: AtomicBool,
    /// A site that locks on sleep is unlocked.
    armed: AtomicBool,
    inhibitor: Mutex<Option<OwnedFd>>,
    requests: Mutex<Option<Sender<bool>>>,
}

impl Shared {
    /// Asks the inhibitor thread to follow `armed`. The flag is read while
    /// the sender is locked, so the last request always carries the newest
    /// value, whichever thread sends.
    fn sync(&self) {
        let sender = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(sender) = sender.as_ref() {
            let _ = sender.send(self.armed.load(Ordering::SeqCst));
        }
    }

    fn release(&self) {
        // Closing the descriptor releases the inhibitor.
        drop(
            self.inhibitor
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
    }

    #[cfg(test)]
    fn holds_inhibitor(&self) -> bool {
        self.inhibitor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }
}

/// The running adapter.
pub(super) struct LogindSignals {
    shared: Arc<Shared>,
}

impl SignalControl for LogindSignals {
    fn set_armed(&self, armed: bool) {
        self.shared.armed.store(armed, Ordering::SeqCst);
        if self.shared.ready.load(Ordering::SeqCst) {
            self.shared.sync();
        }
    }

    fn support(&self) -> SignalSupport {
        SignalSupport {
            sleep: self.shared.sleep.load(Ordering::SeqCst),
            screen_lock: self.shared.screen_lock.load(Ordering::SeqCst),
            background: false,
        }
    }

    fn ready(&self) -> bool {
        self.shared.ready.load(Ordering::SeqCst)
    }
}

/// Starts listening on the system bus (`DBUS_SYSTEM_BUS_ADDRESS` when set).
/// Without a bus the adapter reports no signal; nothing fails.
pub(super) fn start_signal_watch(sink: Weak<dyn SignalSink>) -> Box<dyn SignalControl> {
    Box::new(start(sink))
}

fn start(sink: Weak<dyn SignalSink>) -> LogindSignals {
    let shared = Arc::new(Shared::default());
    let watcher = Arc::clone(&shared);
    let _ = std::thread::Builder::new()
        .name("vault-logind".into())
        .spawn(move || {
            let _ = watch(&watcher, &sink);
            watcher.ready.store(false, Ordering::SeqCst);
            watcher.release();
        });
    LogindSignals { shared }
}

fn watch(shared: &Arc<Shared>, sink: &Weak<dyn SignalSink>) -> zbus::Result<()> {
    let connection = Connection::system()?;
    let (sender, requests) = mpsc::channel();
    *shared
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(sender);
    spawn_inhibitor(Arc::clone(shared), connection.clone(), requests);
    let session = session_path(&connection).ok();
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(LOGIN1)?
        .build();
    let messages = MessageIterator::for_match_rule(rule, &connection, Some(64))?;
    shared.sleep.store(true, Ordering::SeqCst);
    shared
        .screen_lock
        .store(session.is_some(), Ordering::SeqCst);
    shared.ready.store(true, Ordering::SeqCst);
    shared.sync();
    for message in messages {
        let Ok(message) = message else {
            break;
        };
        let Some(sink) = sink.upgrade() else {
            break;
        };
        handle(shared, sink.as_ref(), session.as_ref(), &message);
    }
    Ok(())
}

/// The inhibitor thread: holds or releases the `delay` inhibitor on request.
fn spawn_inhibitor(shared: Arc<Shared>, connection: Connection, requests: Receiver<bool>) {
    let _ = std::thread::Builder::new()
        .name("vault-logind-inhibit".into())
        .spawn(move || {
            for hold in requests {
                let mut held = shared
                    .inhibitor
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if !hold {
                    *held = None;
                } else if held.is_none() {
                    // Without logind's consent the sleep lock still runs,
                    // only without the guarantee to finish before sleep.
                    *held = inhibit(&connection).ok();
                }
            }
        });
}

fn inhibit(connection: &Connection) -> zbus::Result<OwnedFd> {
    let reply = connection.call_method(
        Some(LOGIN1),
        LOGIN1_PATH,
        Some(MANAGER),
        "Inhibit",
        &("sleep", "Smart Explorer", "Tresore sperren", "delay"),
    )?;
    reply.body().deserialize::<OwnedFd>()
}

/// The object of the caller's login session.
fn session_path(connection: &Connection) -> zbus::Result<OwnedObjectPath> {
    let reply = connection.call_method(
        Some(LOGIN1),
        AUTO_SESSION,
        Some(PROPERTIES),
        "Get",
        &(SESSION, "Id"),
    )?;
    let id = String::try_from(reply.body().deserialize::<OwnedValue>()?)?;
    let reply =
        connection.call_method(Some(LOGIN1), LOGIN1_PATH, Some(MANAGER), "GetSession", &id)?;
    reply.body().deserialize::<OwnedObjectPath>()
}

fn on_session(path: &ObjectPath<'_>, session: Option<&OwnedObjectPath>) -> bool {
    session.is_some_and(|session| session.as_str() == path.as_str())
}

/// `PropertiesChanged` of the session with `LockedHint` = true.
fn locked_hint_set(message: &Message) -> bool {
    let body = message.body();
    let Ok((interface, changed, _)) =
        body.deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
    else {
        return false;
    };
    interface == SESSION
        && changed
            .get("LockedHint")
            .and_then(|value| bool::try_from(value).ok())
            == Some(true)
}

fn handle(
    shared: &Shared,
    sink: &dyn SignalSink,
    session: Option<&OwnedObjectPath>,
    message: &Message,
) {
    let header = message.header();
    let (Some(path), Some(interface), Some(member)) =
        (header.path(), header.interface(), header.member())
    else {
        return;
    };
    match (interface.as_str(), member.as_str()) {
        (MANAGER, "PrepareForSleep") if path.as_str() == LOGIN1_PATH => {
            match message.body().deserialize::<bool>() {
                Ok(true) => {
                    // Locked first, then sleep may go on.
                    sink.signal(SystemSignal::Sleep);
                    shared.release();
                }
                Ok(false) => {
                    sink.signal(SystemSignal::Resume);
                    shared.sync();
                }
                Err(_) => {}
            }
        }
        (PROPERTIES, "PropertiesChanged") if on_session(path, session) => {
            if locked_hint_set(message) {
                sink.signal(SystemSignal::ScreenLocked);
            }
        }
        (SESSION, "Lock") if on_session(path, session) => sink.signal(SystemSignal::ScreenLocked),
        _ => {}
    }
}

/// Memory available to programs (`MemAvailable`) for the scrypt check of
/// C1.3 (c), as `transfer/os/unix.rs` reads it.
pub(super) fn available_memory() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text
        .lines()
        .find(|line| line.starts_with("MemAvailable:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

#[cfg(test)]
#[path = "linux_logind_tests.rs"]
mod logind_tests;
