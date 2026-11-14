//! Measurement switches of the purpose profiles (plan 2026-11-14 C7, M1 to
//! M3): only in tests and the `test-support` feature, never a setting.
//!
//! A switch holds for connections made while its guard lives. The guard also
//! serializes every user of the switches in one test binary, so a test that
//! reads the plan defaults takes a guard with [`ProfileOverrides::default`].
use std::num::NonZeroUsize;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

/// Deviations from the plan defaults; `None` keeps the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProfileOverrides {
    /// `Some(false)`: file connections use the plain inner controller
    /// (Cubic without yielding, "vorher" of M2).
    pub(crate) yielding: Option<bool>,
    /// Queueing delay target `T` of the yielding controller (M2: 15, 25, 50 ms).
    pub(crate) target: Option<Duration>,
    /// Segments per transmit and datagrams per driver pass of the bulk
    /// purposes `Files` and `Tcp` (M1: 10/20 against 40/80).
    pub(crate) transmit: Option<(NonZeroUsize, NonZeroUsize)>,
    /// `Some(true)`: the bulk purposes request the ACK frequency extension
    /// with its defaults (M1/M9).
    pub(crate) ack_frequency: Option<bool>,
    /// `Some(false)`: transfer streams keep priority 0 ("vorher" of M3).
    pub(crate) transfer_priority: Option<bool>,
}

static ACTIVE: Mutex<ProfileOverrides> = Mutex::new(ProfileOverrides {
    yielding: None,
    target: None,
    transmit: None,
    ack_frequency: None,
    transfer_priority: None,
});
static SERIAL: Mutex<()> = Mutex::new(());

/// The switches in force.
pub(crate) fn current() -> ProfileOverrides {
    *ACTIVE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Holds `overrides` until the guard drops; then the plan defaults return.
pub(crate) struct OverrideGuard {
    _serial: MutexGuard<'static, ()>,
}

impl Drop for OverrideGuard {
    fn drop(&mut self) {
        *ACTIVE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = ProfileOverrides::default();
    }
}

/// Sets `overrides` for connections made while the guard lives; waits for
/// any other guard of this process first.
pub(crate) fn hold(overrides: ProfileOverrides) -> OverrideGuard {
    let serial = SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *ACTIVE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = overrides;
    OverrideGuard { _serial: serial }
}
