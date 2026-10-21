//! When a vault locks by itself (C3.3, C3.4): the lock reasons, the system
//! signals the platform adapters report, and the decisions over the lock
//! settings of a site. Pure decisions over typed facts: the session register
//! (`vault/os/shared/session.rs`) measures time and activity, the adapters
//! under `vault/os/` report the signals, and both ask here.
use std::time::Duration;

use super::registry_types::LockSettings;

/// Idle minutes when the idle lock is switched on without a value (C3.3:
/// off by default, 30 minutes, as Cryptomator Desktop
/// `DEFAULT_AUTOLOCK_WHEN_IDLE`).
pub(crate) const DEFAULT_IDLE_MINUTES: u32 = 30;
/// The background delays Android offers in minutes, `0` = at once (C3.3,
/// as Cryptomator Android `LockTimeout`: instant, 1, 2, 5, 10 minutes;
/// „aus“ is `None`).
pub(crate) const BACKGROUND_CHOICES: [u32; 5] = [0, 1, 2, 5, 10];

/// Which lock triggers a platform has (C7.1: sleep and screen lock only on
/// the desktop, screen off and background only on Android).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LockPlatform {
    Desktop,
    Android,
}

/// Why a vault was locked; travels in events and status replies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LockReason {
    /// `se vault lock`, the lock button, „Alle sperren“.
    Explicit,
    /// No vault operation and no open file for the idle minutes.
    Idle,
    /// The system goes to sleep or hibernation (desktop).
    Sleep,
    /// The session's screen was locked (desktop).
    ScreenLock,
    /// The screen went off (Android).
    ScreenOff,
    /// The app stayed in the background (Android).
    Background,
    /// A drive of the vault ended and nothing else holds the session.
    MountEnd,
    /// The worker that holds the keys ends.
    WorkerEnd,
    /// The site is being removed from the vault list.
    Removed,
}

impl LockReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Idle => "idle",
            Self::Sleep => "sleep",
            Self::ScreenLock => "screen_lock",
            Self::ScreenOff => "screen_off",
            Self::Background => "background",
            Self::MountEnd => "mount_end",
            Self::WorkerEnd => "worker_end",
            Self::Removed => "removed",
        }
    }

    /// A forced lock does not wait for unsaved drive changes: they stay
    /// sealed (C3.3). Only a lock someone asked for can be refused.
    pub(crate) fn forced(self) -> bool {
        !matches!(self, Self::Explicit | Self::Removed)
    }

    /// How long the lock participants may take (C3.3 „Sperrteilnehmer mit
    /// Frist“). Before sleep logind waits for a delay inhibitor at most
    /// `InhibitDelayMaxSec` (5 s by default), so the participants get 4 s;
    /// a lock someone asked for may publish drive changes first.
    pub(crate) fn participant_budget(self) -> Duration {
        match self {
            Self::Sleep => Duration::from_secs(4),
            Self::Explicit | Self::Removed => Duration::from_secs(60),
            _ => Duration::from_secs(15),
        }
    }
}

/// What a platform adapter reports (C3.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SystemSignal {
    /// Windows `PBT_APMSUSPEND`, logind `PrepareForSleep(true)`.
    Sleep,
    /// logind `PrepareForSleep(false)`.
    Resume,
    /// Windows `WTS_SESSION_LOCK`, logind `LockedHint` or `Lock`.
    ScreenLocked,
    /// Android `screenOn = false` (`sys.hostState`).
    ScreenOff,
}

/// Receives the signals of a platform adapter (the session register).
pub(crate) trait SignalSink: Send + Sync {
    fn signal(&self, signal: SystemSignal);
}

/// Which signals a platform adapter actually receives; surfaces show the
/// switches of the others as „auf diesem System nicht verfügbar“.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SignalSupport {
    pub(crate) sleep: bool,
    pub(crate) screen_lock: bool,
    pub(crate) background: bool,
}

/// The running platform adapter, as the session register controls it.
pub(crate) trait SignalControl: Send + Sync {
    /// At least one unlocked site locks on sleep: Linux then holds a logind
    /// `delay` inhibitor, so the lock finishes before the system sleeps.
    fn set_armed(&self, armed: bool);
    fn support(&self) -> SignalSupport;
    /// The adapter listens (tests wait for it before they send signals).
    fn ready(&self) -> bool {
        true
    }
}

/// What the register measured for one unlocked site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SiteClock {
    /// Time since the last vault operation; zero while a file is open.
    pub(crate) idle: Duration,
    /// Time the app has spent in the background (Android); `None` while it
    /// is visible and while a file of the site is open (U31 S8: the register
    /// leaves it out then, so `due` and `wake_in` never count it).
    pub(crate) background: Option<Duration>,
}

/// The lock settings of a new site on this platform (C3.3).
pub(crate) fn platform_defaults(platform: LockPlatform) -> LockSettings {
    match platform {
        LockPlatform::Desktop => LockSettings::desktop(),
        LockPlatform::Android => LockSettings::android(),
    }
}

fn minutes(count: u32) -> Duration {
    Duration::from_secs(u64::from(count) * 60)
}

/// The reason a site locks now, if any.
pub(crate) fn due(
    settings: &LockSettings,
    clock: SiteClock,
    platform: LockPlatform,
) -> Option<LockReason> {
    if let Some(idle) = settings.auto_lock_idle_min {
        if clock.idle >= minutes(idle) {
            return Some(LockReason::Idle);
        }
    }
    if platform == LockPlatform::Android {
        if let (Some(limit), Some(background)) = (settings.lock_on_background_min, clock.background)
        {
            if background >= minutes(limit) {
                return Some(LockReason::Background);
            }
        }
    }
    None
}

/// How long until `due` can say yes without new activity; `None` = never.
pub(crate) fn wake_in(
    settings: &LockSettings,
    clock: SiteClock,
    platform: LockPlatform,
) -> Option<Duration> {
    let mut next: Option<Duration> = None;
    let mut consider = |left: Duration| next = Some(next.map_or(left, |known| known.min(left)));
    if let Some(idle) = settings.auto_lock_idle_min {
        consider(minutes(idle).saturating_sub(clock.idle));
    }
    if platform == LockPlatform::Android {
        if let (Some(limit), Some(background)) = (settings.lock_on_background_min, clock.background)
        {
            consider(minutes(limit).saturating_sub(background));
        }
    }
    next
}

/// Whether `signal` locks a site with these settings on this platform.
pub(crate) fn locks_on(
    settings: &LockSettings,
    signal: SystemSignal,
    platform: LockPlatform,
) -> bool {
    match (signal, platform) {
        (SystemSignal::Sleep, LockPlatform::Desktop) => settings.lock_on_sleep,
        (SystemSignal::ScreenLocked, LockPlatform::Desktop) => settings.lock_on_screen_lock,
        // On Android the same switch means „Bildschirm aus“ (C2.1, C3.3).
        (SystemSignal::ScreenOff, LockPlatform::Android) => settings.lock_on_screen_lock,
        _ => false,
    }
}

/// The idle minutes after switching the idle lock on or off: a given value,
/// else 30 minutes (C3.3).
pub(crate) fn idle_minutes(enabled: bool, minutes: Option<u32>) -> Result<Option<u32>, String> {
    if !enabled {
        return Ok(None);
    }
    let value = minutes.unwrap_or(DEFAULT_IDLE_MINUTES);
    check_idle(Some(value))?;
    Ok(Some(value))
}

/// Idle minutes start at 1: zero would lock at once, which is what „Sperren“
/// does. There is no upper limit.
pub(crate) fn check_idle(minutes: Option<u32>) -> Result<(), String> {
    match minutes {
        Some(0) => Err("Sperren nach Leerlauf braucht mindestens 1 Minute (oder „aus“)".into()),
        _ => Ok(()),
    }
}

/// The background delay is one of the choices of C3.3 or off.
pub(crate) fn check_background(minutes: Option<u32>) -> Result<(), String> {
    match minutes {
        Some(value) if !BACKGROUND_CHOICES.contains(&value) => Err(format!(
            "Sperren im Hintergrund: sofort (0), 1, 2, 5 oder 10 Minuten oder aus, nicht {value}"
        )),
        _ => Ok(()),
    }
}

/// Checks a complete set of lock settings.
pub(crate) fn check_settings(settings: &LockSettings) -> Result<(), String> {
    check_idle(settings.auto_lock_idle_min)?;
    check_background(settings.lock_on_background_min)
}
