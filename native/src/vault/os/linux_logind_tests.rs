//! The wiring of the Linux lock signals against a test logind on a private
//! system bus (stage `vault-linux`, fixture `bus`: `DBUS_SYSTEM_BUS_ADDRESS`
//! of a bus anyone may own names on, `SE_TASK_PRIVATE_SYSTEM_BUS=1`).
//! Whether GNOME and KDE set `LockedHint` themselves stays a hand test
//! (assumption A2, S3 K13: `vault_task_screen_lock_delivery_by_os`).
use std::collections::HashMap;
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use zbus::zvariant::{OwnedFd, OwnedObjectPath, Value};

use super::super::session_test_support::{
    is_unlocked, settings, test_register, unlock_again, unlocked_site, wait_until,
};
use super::{start, LOGIN1, LOGIN1_PATH, MANAGER, PROPERTIES, SESSION};
use crate::vault::core::lock_policy::{LockPlatform, SignalControl, SignalSink};

const SESSION_ID: &str = "c7";

fn session_object() -> String {
    format!("/org/freedesktop/login1/session/{SESSION_ID}")
}

#[derive(Default)]
struct Inhibitors {
    calls: Vec<String>,
    /// Our ends of the descriptors handed out; EOF = released.
    held: Vec<UnixStream>,
}

struct FakeManager {
    inhibitors: Arc<Mutex<Inhibitors>>,
}

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl FakeManager {
    fn inhibit(
        &self,
        what: String,
        who: String,
        why: String,
        mode: String,
    ) -> zbus::fdo::Result<OwnedFd> {
        let (ours, theirs) =
            UnixStream::pair().map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        let mut inhibitors = self
            .inhibitors
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        inhibitors.calls.push(format!("{what}|{who}|{why}|{mode}"));
        inhibitors.held.push(ours);
        Ok(OwnedFd::from(std::os::fd::OwnedFd::from(theirs)))
    }

    fn get_session(&self, id: String) -> zbus::fdo::Result<OwnedObjectPath> {
        if id != SESSION_ID {
            return Err(zbus::fdo::Error::Failed(format!("keine Sitzung {id}")));
        }
        OwnedObjectPath::try_from(session_object())
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }
}

struct FakeSession {
    locked: Arc<AtomicBool>,
}

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl FakeSession {
    #[zbus(property)]
    fn id(&self) -> String {
        SESSION_ID.to_string()
    }

    #[zbus(property)]
    fn locked_hint(&self) -> bool {
        self.locked.load(Ordering::SeqCst)
    }
}

fn test_logind(
    inhibitors: &Arc<Mutex<Inhibitors>>,
    locked: &Arc<AtomicBool>,
) -> zbus::Result<zbus::blocking::Connection> {
    zbus::blocking::connection::Builder::system()?
        .name(LOGIN1)?
        .serve_at(
            LOGIN1_PATH,
            FakeManager {
                inhibitors: Arc::clone(inhibitors),
            },
        )?
        .serve_at(
            "/org/freedesktop/login1/session/auto",
            FakeSession {
                locked: Arc::clone(locked),
            },
        )?
        .serve_at(
            session_object(),
            FakeSession {
                locked: Arc::clone(locked),
            },
        )?
        .build()
}

/// Whether the other end of `stream` is closed everywhere.
fn released(stream: &mut UnixStream) -> bool {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let mut byte = [0u8; 1];
    matches!(stream.read(&mut byte), Ok(0))
}

#[test]
#[ignore = "braucht den privaten Systembus der Stufe vault-linux (fixture bus)"]
fn vault_task_linux_logind_sleep_and_locked_hint_lock() {
    assert_eq!(
        std::env::var("SE_TASK_PRIVATE_SYSTEM_BUS").as_deref(),
        Ok("1"),
        "nur auf dem privaten Systembus der Suite (fixture bus), nie auf dem des Rechners"
    );
    let inhibitors = Arc::new(Mutex::new(Inhibitors::default()));
    let locked = Arc::new(AtomicBool::new(false));
    let logind = test_logind(&inhibitors, &locked).expect("Test-logind auf dem privaten Bus");
    let register = test_register(LockPlatform::Desktop);
    let on_sleep = unlocked_site(&register, settings(None, true, false, None));
    let on_screen = unlocked_site(&register, settings(None, false, true, None));
    let neither = unlocked_site(&register, settings(None, false, false, None));
    let sink: Weak<dyn SignalSink> = Arc::downgrade(&register);
    let signals = start(sink);
    wait_until("Signale von logind abonniert", || signals.ready());
    let support = signals.support();
    assert!(support.sleep && support.screen_lock, "{support:?}");

    // A site locks on sleep: the worker holds a `delay` inhibitor.
    signals.set_armed(true);
    wait_until("delay-Inhibitor gehalten", || {
        signals.shared.holds_inhibitor()
    });
    let first = inhibitors.lock().unwrap().calls.first().cloned();
    assert_eq!(
        first.as_deref(),
        Some("sleep|Smart Explorer|Tresore sperren|delay")
    );

    // PrepareForSleep(true): locked first, then the inhibitor is released.
    logind
        .emit_signal(None::<&str>, LOGIN1_PATH, MANAGER, "PrepareForSleep", &true)
        .expect("PrepareForSleep(true)");
    wait_until("gesperrt beim Ruhezustand", || {
        !is_unlocked(&register, &on_sleep.0)
    });
    assert!(on_sleep.1.is_locked(), "key wiped before sleep");
    wait_until("Inhibitor freigegeben", || {
        !signals.shared.holds_inhibitor()
    });
    let mut ours = inhibitors.lock().unwrap().held.remove(0);
    wait_until("Deskriptor des Inhibitors geschlossen", || {
        released(&mut ours)
    });
    assert!(is_unlocked(&register, &on_screen.0) && is_unlocked(&register, &neither.0));

    // PrepareForSleep(false): held again while a site locks on sleep.
    logind
        .emit_signal(
            None::<&str>,
            LOGIN1_PATH,
            MANAGER,
            "PrepareForSleep",
            &false,
        )
        .expect("PrepareForSleep(false)");
    wait_until("Inhibitor nach dem Aufwachen erneut gehalten", || {
        signals.shared.holds_inhibitor()
    });
    assert_eq!(inhibitors.lock().unwrap().calls.len(), 2);

    // LockedHint = true on the session object locks the screen lock sites.
    locked.store(true, Ordering::SeqCst);
    let mut changed: HashMap<&str, Value<'_>> = HashMap::new();
    changed.insert("LockedHint", Value::from(true));
    logind
        .emit_signal(
            None::<&str>,
            session_object().as_str(),
            PROPERTIES,
            "PropertiesChanged",
            &(SESSION, changed, Vec::<&str>::new()),
        )
        .expect("PropertiesChanged");
    wait_until("gesperrt bei LockedHint", || {
        !is_unlocked(&register, &on_screen.0)
    });

    // The signal `Lock` of the session does it as well.
    let again = unlock_again(&register, &on_screen.0, settings(None, false, true, None));
    logind
        .emit_signal(
            None::<&str>,
            session_object().as_str(),
            SESSION,
            "Lock",
            &(),
        )
        .expect("Lock");
    wait_until("gesperrt beim Signal Lock", || again.is_locked());

    // `Lock` of another session locks nothing.
    let other = unlocked_site(&register, settings(None, false, true, None));
    logind
        .emit_signal(
            None::<&str>,
            "/org/freedesktop/login1/session/c8",
            SESSION,
            "Lock",
            &(),
        )
        .expect("Lock of another session");
    std::thread::sleep(Duration::from_millis(300));
    assert!(is_unlocked(&register, &other.0));
    assert!(is_unlocked(&register, &neither.0));
}
