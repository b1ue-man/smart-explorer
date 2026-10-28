//! Public, address-free route state shared by every Share payload protocol.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RouteClass {
    Lan,
    Direct,
    Relay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RoutePhase {
    Collecting,
    CheckingLan,
    CheckingDirect,
    Lan,
    Direct,
    /// Payload over a validated relay while LAN/direct paths are still being
    /// established (start bridge, `route_start.rs`); moves up in place.
    RelayStart,
    RelayFallback,
    Paused,
    Offline,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RemoteRouteGuarantee {
    /// No application negotiation has yet proved the remote sender's policy.
    Unconfirmed,
    BothSenders,
    /// The local sender enforces the policy; an older remote sender cannot be controlled.
    LocalSenderOnly,
}

/// On the IPC wire `RelayStart` is written as `relay_fallback` with
/// `relay_start: true` (review C1): a 0.5.188 GUI reading a newer worker's
/// login session then parses a known phase, and this build restores
/// `RelayStart` from the flag.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(from = "RouteStatusWire", into = "RouteStatusWire")]
pub(crate) struct RouteStatus {
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    /// Increases on each interruption; screen consumers discard dependent frames.
    pub(crate) pause_epoch: u64,
    pub(crate) phase: RoutePhase,
    pub(crate) class: Option<RouteClass>,
    pub(crate) application_allowed: bool,
    pub(crate) reason: String,
    pub(crate) remote_guarantee: RemoteRouteGuarantee,
}

#[derive(Serialize, Deserialize)]
struct RouteStatusWire {
    generation: u64,
    revision: u64,
    pause_epoch: u64,
    phase: RoutePhase,
    class: Option<RouteClass>,
    application_allowed: bool,
    reason: String,
    remote_guarantee: RemoteRouteGuarantee,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    relay_start: bool,
}
impl From<RouteStatus> for RouteStatusWire {
    fn from(status: RouteStatus) -> Self {
        let relay_start = status.phase == RoutePhase::RelayStart;
        Self {
            generation: status.generation,
            revision: status.revision,
            pause_epoch: status.pause_epoch,
            phase: if relay_start {
                RoutePhase::RelayFallback
            } else {
                status.phase
            },
            class: status.class,
            application_allowed: status.application_allowed,
            reason: status.reason,
            remote_guarantee: status.remote_guarantee,
            relay_start,
        }
    }
}
impl From<RouteStatusWire> for RouteStatus {
    fn from(wire: RouteStatusWire) -> Self {
        let relay_start = wire.relay_start && wire.phase == RoutePhase::RelayFallback;
        Self {
            generation: wire.generation,
            revision: wire.revision,
            pause_epoch: wire.pause_epoch,
            phase: if relay_start {
                RoutePhase::RelayStart
            } else {
                wire.phase
            },
            class: wire.class,
            application_allowed: wire.application_allowed,
            reason: wire.reason,
            remote_guarantee: wire.remote_guarantee,
        }
    }
}

impl RouteStatus {
    pub(crate) fn initial(generation: u64) -> Self {
        Self {
            generation,
            revision: 0,
            pause_epoch: 0,
            phase: RoutePhase::Collecting,
            class: None,
            application_allowed: false,
            reason: "Verbindung wird ermittelt".into(),
            remote_guarantee: RemoteRouteGuarantee::Unconfirmed,
        }
    }

    pub(crate) fn label(&self) -> &'static str {
        match self.phase {
            RoutePhase::Collecting => "Verbindung wird ermittelt",
            RoutePhase::CheckingLan => "LAN wird geprüft",
            RoutePhase::CheckingDirect => "Direktverbindung wird geprüft",
            RoutePhase::Lan => "LAN/WLAN",
            RoutePhase::Direct => "Direkt",
            RoutePhase::RelayStart => "über Relay, Direktweg wird aufgebaut",
            RoutePhase::RelayFallback => "über Relay (Rückfall)",
            RoutePhase::Paused => "pausiert wegen Pfadwechsel",
            RoutePhase::Offline => "offline",
            RoutePhase::Cancelled => "abgebrochen",
        }
    }

    pub(crate) fn remote_limit(&self) -> Option<&'static str> {
        (self.remote_guarantee == RemoteRouteGuarantee::LocalSenderOnly)
            .then_some("Direktreihenfolge der Gegenstelle nicht durchsetzbar; Update erforderlich")
    }
}
