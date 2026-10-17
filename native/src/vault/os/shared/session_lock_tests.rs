//! The lock triggers, the lock participants and the drive-end rule of the
//! session register (VLT-M4, stage `vault_unit`, C3.3).
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::vault::core::lock_policy::{
    check_background, due, idle_minutes, locks_on, platform_defaults, wake_in, LockPlatform,
    LockReason, SignalSink, SiteClock, SystemSignal,
};
use crate::vault::core::registry_types::LockSettings;
use crate::vault::core::VaultSession;

use super::session::{
    ConsumerKind, LockError, LockParticipant, LockRequest, SessionRegister, SiteState,
};
use super::session_test_support::{
    is_unlocked, settings, test_register, unlock_again, unlocked_site,
};

const MINUTE: u64 = 60;

fn clock(idle: u64, background: Option<u64>) -> SiteClock {
    SiteClock {
        idle: Duration::from_secs(idle),
        background: background.map(Duration::from_secs),
    }
}

fn last_reason(register: &SessionRegister, site: &str) -> Option<LockReason> {
    let (_, events) = register.events();
    events
        .iter()
        .rev()
        .find(|event| event.site == site && event.state == SiteState::Locked)
        .and_then(|event| event.reason)
}

#[test]
fn vault_task_lock_policy_triggers_and_defaults() {
    use LockPlatform::{Android, Desktop};
    // Defaults (C3.3): desktop idle off, sleep on, screen lock off;
    // Android idle off, screen off on, background 1 minute.
    let desktop = platform_defaults(Desktop);
    let android = platform_defaults(Android);
    assert_eq!(desktop, settings(None, true, false, None));
    assert_eq!(android, settings(None, false, true, Some(1)));
    assert_eq!(
        super::platform_lock_defaults(),
        platform_defaults(super::LOCK_PLATFORM)
    );
    // Idle switched on without a value: 30 minutes; at least 1 minute.
    assert_eq!(idle_minutes(true, None), Ok(Some(30)));
    assert_eq!(idle_minutes(true, Some(5)), Ok(Some(5)));
    assert_eq!(idle_minutes(false, Some(5)), Ok(None));
    assert!(idle_minutes(true, Some(0)).is_err());
    // Background: at once, 1, 2, 5, 10 minutes or off.
    for minutes in [0, 1, 2, 5, 10] {
        assert_eq!(check_background(Some(minutes)), Ok(()));
    }
    assert_eq!(check_background(None), Ok(()));
    assert!(check_background(Some(3)).is_err() && check_background(Some(30)).is_err());

    // The decisions.
    let idle30 = settings(Some(30), false, false, None);
    assert_eq!(due(&idle30, clock(30 * MINUTE - 1, None), Desktop), None);
    assert_eq!(
        due(&idle30, clock(30 * MINUTE, None), Desktop),
        Some(LockReason::Idle)
    );
    assert_eq!(due(&desktop, clock(10_000 * MINUTE, None), Desktop), None);
    assert_eq!(
        wake_in(&idle30, clock(10 * MINUTE, None), Desktop),
        Some(Duration::from_secs(20 * MINUTE))
    );
    assert_eq!(wake_in(&desktop, clock(0, None), Desktop), None);
    assert_eq!(due(&android, clock(0, Some(59)), Android), None);
    assert_eq!(
        due(&android, clock(0, Some(60)), Android),
        Some(LockReason::Background)
    );
    assert_eq!(due(&android, clock(0, None), Android), None, "visible app");
    assert_eq!(
        due(&android, clock(0, Some(3600)), Desktop),
        None,
        "no background on the desktop"
    );
    let at_once = settings(None, false, true, Some(0));
    assert_eq!(
        due(&at_once, clock(0, Some(0)), Android),
        Some(LockReason::Background)
    );
    assert!(locks_on(&desktop, SystemSignal::Sleep, Desktop));
    assert!(!locks_on(&desktop, SystemSignal::ScreenLocked, Desktop));
    assert!(locks_on(
        &settings(None, false, true, None),
        SystemSignal::ScreenLocked,
        Desktop
    ));
    assert!(locks_on(&android, SystemSignal::ScreenOff, Android));
    assert!(!locks_on(&android, SystemSignal::Sleep, Android));
    assert!(!locks_on(&android, SystemSignal::ScreenLocked, Android));
    assert!(!LockReason::Explicit.forced() && !LockReason::Removed.forced());
    assert!(LockReason::Idle.forced() && LockReason::Sleep.forced());
    assert_eq!(
        LockReason::Sleep.participant_budget(),
        Duration::from_secs(4)
    );

    // The timer of the register with a clock set ahead.
    let register = test_register(Desktop);
    let (idle_site, _) = unlocked_site(&register, idle30);
    let (never, _) = unlocked_site(&register, desktop);
    let start = Instant::now();
    register.tick(start + Duration::from_secs(29 * MINUTE));
    assert!(is_unlocked(&register, &idle_site));
    register.tick(start + Duration::from_secs(31 * MINUTE));
    assert!(!is_unlocked(&register, &idle_site) && is_unlocked(&register, &never));
    assert_eq!(last_reason(&register, &idle_site), Some(LockReason::Idle));
    // An open file is no idle time; the clock starts when it closes.
    let (held, held_session) = unlocked_site(&register, settings(Some(1), false, false, None));
    let stream = held_session.cryptor();
    register.tick(Instant::now() + Duration::from_secs(10 * MINUTE));
    assert!(is_unlocked(&register, &held));
    drop(stream);
    register.tick(Instant::now() + Duration::from_secs(30));
    assert!(is_unlocked(&register, &held));
    register.tick(Instant::now() + Duration::from_secs(MINUTE + 1));
    assert!(!is_unlocked(&register, &held));
    // The sleep signal of a desktop.
    register.signal(SystemSignal::Sleep);
    assert!(!is_unlocked(&register, &never));
    assert_eq!(last_reason(&register, &never), Some(LockReason::Sleep));

    // Android: background minutes and screen off through `sys.hostState`.
    let phone = test_register(Android);
    let (background, _) = unlocked_site(&phone, settings(None, false, false, Some(1)));
    let (screen, _) = unlocked_site(&phone, settings(None, false, true, None));
    let (keep, _) = unlocked_site(&phone, settings(None, false, false, None));
    let hidden = Instant::now();
    phone.set_host(false, true, hidden);
    phone.tick(hidden + Duration::from_secs(59));
    assert!(is_unlocked(&phone, &background));
    phone.tick(hidden + Duration::from_secs(60));
    assert!(!is_unlocked(&phone, &background));
    assert_eq!(
        last_reason(&phone, &background),
        Some(LockReason::Background)
    );
    phone.set_host(true, true, Instant::now());
    assert!(is_unlocked(&phone, &screen));
    phone.set_host(true, false, Instant::now());
    assert!(!is_unlocked(&phone, &screen) && is_unlocked(&phone, &keep));
    assert_eq!(last_reason(&phone, &screen), Some(LockReason::ScreenOff));
}

/// Records what it saw at the time of the call.
#[derive(Default)]
struct Recorder {
    sessions: Mutex<HashMap<String, Arc<VaultSession>>>,
    /// (site, reason, keep_unsaved, key wiped at the call, deadline ahead)
    calls: Mutex<Vec<(String, LockReason, bool, bool, bool)>>,
    refuse: AtomicBool,
}

impl Recorder {
    fn watch(&self, site: &str, session: &Arc<VaultSession>) {
        self.sessions
            .lock()
            .unwrap()
            .insert(site.to_string(), Arc::clone(session));
    }

    fn calls(&self) -> Vec<(String, LockReason, bool, bool, bool)> {
        self.calls.lock().unwrap().clone()
    }
}

impl LockParticipant for Recorder {
    fn prepare_lock(&self, site: &str, request: &LockRequest) -> Result<(), String> {
        let wiped = self.sessions.lock().unwrap()[site].is_locked();
        let ahead = request.deadline > Instant::now();
        self.calls.lock().unwrap().push((
            site.to_string(),
            request.reason,
            request.keep_unsaved,
            wiped,
            ahead,
        ));
        if self.refuse.load(Ordering::SeqCst) {
            Err("2 Dateien auf Laufwerk T:".into())
        } else {
            Ok(())
        }
    }
}

#[test]
fn vault_task_lock_participants_called_before_zeroize() {
    let register = test_register(LockPlatform::Desktop);
    let (site, session) = unlocked_site(&register, settings(None, true, false, None));
    let (first, second) = (Arc::new(Recorder::default()), Arc::new(Recorder::default()));
    for recorder in [&first, &second] {
        recorder.watch(&site, &session);
    }
    let first_id = register.add_participant(first.clone());
    register.add_participant(second.clone());

    // „Speichern und sperren“ that cannot save stops the lock: nothing wiped.
    second.refuse.store(true, Ordering::SeqCst);
    let refused = register
        .lock(&site, LockRequest::new(LockReason::Explicit, false))
        .unwrap_err();
    assert_eq!(
        refused,
        LockError::Unsaved("2 Dateien auf Laufwerk T:".into())
    );
    assert_eq!(refused.code(), "vault_unsaved_changes");
    assert!(!session.is_locked() && is_unlocked(&register, &site));
    let call = (site.clone(), LockReason::Explicit, false, false, true);
    assert_eq!(
        (first.calls(), second.calls()),
        (vec![call.clone()], vec![call])
    );

    // „Sperren, Änderungen verschlüsselt aufbewahren“: every participant
    // runs before the key is wiped, then the vault locks.
    assert_eq!(
        register.lock(&site, LockRequest::new(LockReason::Explicit, true)),
        Ok(true)
    );
    assert!(session.is_locked());
    let kept = (site.clone(), LockReason::Explicit, true, false, true);
    assert_eq!(first.calls().last(), Some(&kept));
    assert_eq!(second.calls().last(), Some(&kept));

    // Forced locks never wait for unsaved work; still participants first.
    let again = unlock_again(&register, &site, settings(None, true, false, None));
    for recorder in [&first, &second] {
        recorder.watch(&site, &again);
    }
    register.signal(SystemSignal::Sleep);
    assert!(again.is_locked());
    let forced = (site.clone(), LockReason::Sleep, true, false, true);
    assert_eq!(second.calls().last(), Some(&forced));

    // A removed participant is not asked again.
    register.remove_participant(first_id);
    second.refuse.store(false, Ordering::SeqCst);
    let third = unlock_again(&register, &site, settings(None, true, false, None));
    second.watch(&site, &third);
    let before = first.calls().len();
    assert_eq!(
        register.lock(&site, LockRequest::new(LockReason::Explicit, false)),
        Ok(true)
    );
    assert_eq!(first.calls().len(), before);
    // The events show the stopped lock as „unlocked“ again.
    let (_, events) = register.events();
    let states: Vec<SiteState> = events.iter().map(|event| event.state).collect();
    assert_eq!(
        states[..4],
        [
            SiteState::Unlocked,
            SiteState::Locking,
            SiteState::Unlocked,
            SiteState::Locking
        ]
    );
}

#[test]
fn vault_task_mount_end_locks_only_without_consumers() {
    let register = test_register(LockPlatform::Desktop);
    let plain = LockSettings::desktop();
    // Only the drive holds the session: its end locks the vault.
    let (alone, alone_session) = unlocked_site(&register, plain);
    let drive = register
        .consume(&alone, ConsumerKind::Drive("drive-1".into()))
        .expect("drive consumer");
    assert!(register.mount_ended(&alone, "drive-1", false));
    assert!(alone_session.is_locked());
    assert_eq!(last_reason(&register, &alone), Some(LockReason::MountEnd));
    drop(drive);
    assert!(
        !register.mount_ended(&alone, "drive-1", false),
        "locked already"
    );

    // Another consumer (an open tab) keeps it.
    let (shared, shared_session) = unlocked_site(&register, plain);
    let _drive = register.consume(&shared, ConsumerKind::Drive("drive-2".into()));
    let tab = register.consume(&shared, ConsumerKind::Open).expect("tab");
    assert!(!register.mount_ended(&shared, "drive-2", false));
    assert!(!shared_session.is_locked());
    drop(tab);
    assert!(!shared_session.is_locked(), "only a drive end locks");

    // A second drive of the same site keeps it until it ends too.
    let (two, two_session) = unlocked_site(&register, plain);
    let _a = register.consume(&two, ConsumerKind::Drive("a".into()));
    let _b = register.consume(&two, ConsumerKind::Drive("b".into()));
    assert!(!register.mount_ended(&two, "a", false));
    assert!(!two_session.is_locked());
    assert!(register.mount_ended(&two, "b", false));
    assert!(two_session.is_locked());

    // A drive that ended because of a lock does not lock once more.
    let (by_lock, by_lock_session) = unlocked_site(&register, plain);
    let _c = register.consume(&by_lock, ConsumerKind::Drive("c".into()));
    assert!(!register.mount_ended(&by_lock, "c", true));
    assert!(!by_lock_session.is_locked());
    // A locked site has no consumers to add.
    assert!(register.consume(&alone, ConsumerKind::Open).is_none());
}
