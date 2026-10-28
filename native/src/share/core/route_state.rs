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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
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
