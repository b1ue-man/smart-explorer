//! Why and how the system is kept awake (RV1, contract V4).

/// Work that idle sleep must not interrupt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Reason {
    /// A sync job runs (background worker, desktop window, Android facade).
    SyncRun,
    /// This device serves another device's analysis, duplicate search or
    /// transfer.
    PeerService,
    /// This device runs an analysis or duplicate search on another device.
    RemoteTask,
}

impl Reason {
    pub const ALL: [Reason; 3] = [Reason::SyncRun, Reason::PeerService, Reason::RemoteTask];

    /// Short German reason the OS shows (`powercfg /requests`,
    /// `systemd-inhibit --list`).
    pub fn label(self) -> &'static str {
        match self {
            Reason::SyncRun => "Synchronisation läuft",
            Reason::PeerService => "Ein anderes Gerät nutzt Freigaben dieses Geräts",
            Reason::RemoteTask => "Analyse auf einem anderen Gerät läuft",
        }
    }

    pub(crate) fn index(self) -> usize {
        match self {
            Reason::SyncRun => 0,
            Reason::PeerService => 1,
            Reason::RemoteTask => 2,
        }
    }
}

/// What a platform backend achieved for the current holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Applied {
    pub(crate) engaged: bool,
    pub(crate) throttling_off: bool,
    pub(crate) unavailable: Option<String>,
}

impl Applied {
    pub(crate) const NONE: Applied = Applied {
        engaged: false,
        throttling_off: false,
        unavailable: None,
    };
}

/// The live holds and what the operating system made of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeepAwakeStatus {
    /// Live holds per reason, only reasons that have holds.
    pub holds: Vec<(Reason, usize)>,
    /// The operating system keeps the system awake for them.
    pub engaged: bool,
    /// Windows: power throttling (EcoQoS) is switched off for this process.
    pub throttling_off: bool,
    /// Why the operating-system request is missing (no logind, request
    /// refused, no Android hook); callers log it once.
    pub unavailable: Option<String>,
}
