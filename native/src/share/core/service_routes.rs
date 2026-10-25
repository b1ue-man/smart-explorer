//! Address-free, read-only route projection shared by IPC and every user surface.
use std::collections::{BTreeMap, BTreeSet};
use serde::{Deserialize, Serialize};
use super::route_diagnostics::{RouteBudget, RouteDiagnostics, RouteMeasurement};
use super::route_state::{RemoteRouteGuarantee, RouteClass, RoutePhase};
use super::service::ShareService;

pub const ROUTE_PAYLOAD_MEASUREMENT_NOTE: &str = "Frame-Nutzlast: TX zählt serialisierte STREAM-/DATAGRAM-Bytes einschließlich Wiederholungen und verworfener Sendepläne; RX zählt authentifiziert verarbeitete Bytes einschließlich Wiederholungen. Kein bestätigter Nutzdurchsatz und keine Leitungsrate.";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareRouteSnapshot {
    pub peers: Vec<DeviceRouteDiagnostics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRouteDiagnostics {
    /// Exact identity selectors, never shown as ordinary settings text.
    pub device_id: String,
    pub public_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact_id: Option<String>,
    pub connections: Vec<ConnectionRouteDiagnostics>,
    /// Pair history, not attributed to an ALPN that the source did not record.
    pub measurements: Vec<RouteMeasurementView>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionRouteDiagnostics {
    pub connection_id: String,
    pub protocol: String,
    pub generation: u64,
    pub revision: u64,
    pub pause_epoch: u64,
    pub state: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_class: Option<String>,
    pub application_allowed: bool,
    pub reason: String,
    pub remote_guarantee: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_limit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<RouteBudgetView>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteBudgetView {
    pub initial_pto_ms: u64,
    pub best_pto_ms: u64,
    pub collection_ms: u64,
    pub retry_intervals_ms: Vec<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_probe_attempts: Option<u8>,
    pub response_ms: u64,
    pub validation_ms: u64,
    /// Offset from this network generation's start, never a Unix timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct_deadline_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frozen_candidates: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteMeasurementView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at_unix_ms: Option<u64>,
    pub generation: u64,
    pub path_class: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtt_us: Option<u64>,
    pub serialized_tx_frame_payload_bytes: u64,
    pub authenticated_rx_frame_payload_bytes: u64,
    pub elapsed_ms: u64,
}

impl ShareService {
    pub(crate) fn route_diagnostics(&self) -> ShareRouteSnapshot {
        let state = match self.auth.lock() {
            Ok(state) => state.clone(),
            Err(_) => return ShareRouteSnapshot { error: Some("Routediagnose: Share-Zustand gesperrt".into()), ..Default::default() },
        };
        let mut identities: BTreeMap<(String, String), (Option<String>, BTreeSet<String>)> = BTreeMap::new();
        for contact in &state.direct_contacts {
            if let (Some(device), Some(key)) = (&contact.remote_device_id, &contact.remote_public_key) {
                let row = identities.entry((device.clone(), key.clone())).or_default();
                row.0 = Some(contact.id.clone());
                // ShareIdentity binds its Iroh node id to this pinned public key.
                let node = if contact.expected_node_id.is_empty() { key } else { &contact.expected_node_id };
                row.1.insert(node.clone());
            }
        }
        for grant in &state.direct_grants {
            identities.entry((grant.device_id.clone(), grant.public_key.clone())).or_default().1.insert(grant.node_id.clone());
        }
        for member in state.rooms.iter().flat_map(|room| &room.members) {
            identities.entry((member.device_id.clone(), member.public_key.clone())).or_default().1.insert(member.node_id.clone());
        }
        let mut peers = Vec::new();
        for ((device_id, public_key), (contact_id, nodes)) in identities {
            let mut connections = BTreeMap::new();
            let mut measurements = BTreeMap::new();
            for node in nodes.iter().filter(|node| !node.is_empty()) {
                for facts in super::route_monitor::diagnostics(&self.identity.node_id, Some(node)) {
                    for sample in &facts.measurements { insert_measurement(&mut measurements, sample); }
                    connections.insert(facts.connection_id, ConnectionRouteDiagnostics::from_facts(&facts));
                }
                let pair = super::route_history::pair_key(&self.identity.node_id, node);
                for sample in super::route_history::samples(&pair) { insert_measurement(&mut measurements, &sample); }
            }
            if connections.is_empty() && measurements.is_empty() { continue; }
            peers.push(DeviceRouteDiagnostics { device_id, public_key, contact_id,
                connections: connections.into_values().collect(), measurements: measurements.into_values().collect() });
        }
        ShareRouteSnapshot { peers, ..Default::default() }
    }
}

type MeasurementKey = (u64, u64, String, u64, u64, u64);
fn insert_measurement(rows: &mut BTreeMap<MeasurementKey, RouteMeasurementView>, sample: &RouteMeasurement) {
    let key = (sample.observed_at_unix_ms, sample.generation, class_code(sample.class).into(),
        sample.application_tx_bytes, sample.application_rx_bytes, sample.elapsed_ms);
    rows.entry(key).or_insert_with(|| RouteMeasurementView {
        observed_at_unix_ms: (sample.observed_at_unix_ms > 0).then_some(sample.observed_at_unix_ms),
        generation: sample.generation, path_class: class_code(sample.class).into(), rtt_us: sample.rtt_us,
        serialized_tx_frame_payload_bytes: sample.application_tx_bytes,
        authenticated_rx_frame_payload_bytes: sample.application_rx_bytes, elapsed_ms: sample.elapsed_ms,
    });
}

fn class_code(class: RouteClass) -> &'static str {
    match class { RouteClass::Lan => "lan", RouteClass::Direct => "direct", RouteClass::Relay => "relay" }
}

impl ConnectionRouteDiagnostics {
    fn from_facts(facts: &RouteDiagnostics) -> Self {
        let status = &facts.status;
        Self {
            connection_id: facts.connection_id.to_string(), protocol: facts.protocol.clone(),
            generation: status.generation, revision: status.revision, pause_epoch: status.pause_epoch,
            state: match status.phase {
                RoutePhase::Collecting => "collecting", RoutePhase::CheckingLan => "checking_lan",
                RoutePhase::CheckingDirect => "checking_direct", RoutePhase::Lan => "lan",
                RoutePhase::Direct => "direct", RoutePhase::RelayFallback => "relay_fallback",
                RoutePhase::Paused => "paused", RoutePhase::Offline => "offline", RoutePhase::Cancelled => "cancelled",
            }.into(),
            label: status.label().into(), path_class: status.class.map(|class| class_code(class).into()),
            application_allowed: status.application_allowed, reason: status.reason.clone(),
            remote_guarantee: match status.remote_guarantee {
                RemoteRouteGuarantee::Unconfirmed => "unconfirmed",
                RemoteRouteGuarantee::BothSenders => "both_senders",
                RemoteRouteGuarantee::LocalSenderOnly => "local_sender_only",
            }.into(),
            remote_limit: status.remote_limit().map(str::to_string),
            budget: (facts.budget.initial_pto_ms > 0).then(|| RouteBudgetView::from_facts(&facts.budget)),
        }
    }

    pub fn detail_lines(&self) -> Vec<String> {
        let mut lines = vec![format!("Netzgeneration {}; Revision {}; Pfadwechselstand {}", self.generation, self.revision, self.pause_epoch)];
        if let Some(limit) = &self.remote_limit { lines.push(limit.clone()); }
        if let Some(budget) = &self.budget { lines.extend(budget.detail_lines()); }
        lines
    }
}

impl RouteBudgetView {
    fn from_facts(budget: &RouteBudget) -> Self {
        Self { initial_pto_ms: budget.initial_pto_ms, best_pto_ms: budget.best_pto_ms,
            collection_ms: budget.collection_ms, retry_intervals_ms: budget.retry_intervals_ms.clone(),
            max_probe_attempts: (budget.max_probe_attempts > 0).then_some(budget.max_probe_attempts),
            response_ms: budget.response_ms, validation_ms: budget.validation_ms,
            direct_deadline_ms: (budget.direct_deadline_ms > 0).then_some(budget.direct_deadline_ms),
            frozen_candidates: (budget.direct_deadline_ms > 0).then_some(budget.frozen_candidates) }
    }

    pub fn detail_lines(&self) -> Vec<String> {
        let mut lines = vec![format!("PTO initial/bester: {}/{} ms; Sammelfenster: {} ms", self.initial_pto_ms, self.best_pto_ms, self.collection_ms),
            format!("Antwortbudget: {} ms; Validierungsbudget: {} ms", self.response_ms, self.validation_ms)];
        if let Some(attempts) = self.max_probe_attempts {
            lines.push(format!("Maximale Probeversuche: {attempts}; Retryintervalle: {} ms", self.retry_intervals_ms.iter().map(u64::to_string).collect::<Vec<_>>().join(", ")));
        }
        if let Some(deadline) = self.direct_deadline_ms { lines.push(format!("Direktfrist: {deadline} ms ab Beginn dieser Netzgeneration")); }
        if let Some(candidates) = self.frozen_candidates { lines.push(format!("Eingefrorene Direktkandidaten: {candidates}")); }
        lines
    }
}

impl RouteMeasurementView {
    pub fn detail_line(&self) -> String {
        let class = match self.path_class.as_str() { "lan" => "LAN/WLAN", "direct" => "Direkt", "relay" => "Relay", _ => "unbekannte Pfadklasse" };
        let mut line = format!("{class}; Netzgeneration {}; TX-Frame-Nutzlast {} B; RX-Frame-Nutzlast {} B; Messintervall {} ms", self.generation,
            self.serialized_tx_frame_payload_bytes, self.authenticated_rx_frame_payload_bytes, self.elapsed_ms);
        if let Some(rtt) = self.rtt_us { line.push_str(&format!("; RTT {rtt} µs")); }
        if let Some(at) = self.observed_at_unix_ms { line.push_str(&format!("; Zeitstempel {at} ms seit Unix-Epoche")); }
        line
    }
}
