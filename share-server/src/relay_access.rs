//! Who may use the Iroh relay (B20, S48): endpoints registered at this
//! server's signaling — with a proven key, or the key an older client names
//! in its Hello (the relay handshake proves it anyway) — and, for a short
//! grace period, endpoints that just left (reconnects). Capacity is capped in
//! total and per endpoint.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use iroh_base::EndpointId;
use iroh_relay::server::{
    Access, AccessControl, ClientPingSchedule, ClientRequest, ConnectionId,
};

const RELAY_CAPACITY_DENIAL: &str = "relay connection capacity reached";
const RELAY_SIGNALING_DENIAL: &str = "not registered at this server's signaling";
/// A reconnecting app keeps its relay meanwhile.
const SIGNED_OUT_GRACE: Duration = Duration::from_secs(10 * 60);
/// Bound of the grace list (endpoints that left within the grace period).
const MAX_GRACE_ENTRIES: usize = 65_536;

/// Endpoints currently registered at the signaling, with a grace period.
#[derive(Debug, Default)]
pub(super) struct RelayAdmissions {
    endpoints: Mutex<HashMap<EndpointId, Admission>>,
}

#[derive(Debug)]
struct Admission {
    active: usize,
    left: Option<Instant>,
}

impl RelayAdmissions {
    fn lock(&self) -> MutexGuard<'_, HashMap<EndpointId, Admission>> {
        self.endpoints
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn signed_in(&self, endpoint: EndpointId) {
        let mut endpoints = self.lock();
        if endpoints.len() >= MAX_GRACE_ENTRIES && !endpoints.contains_key(&endpoint) {
            let now = Instant::now();
            endpoints.retain(|_, entry| entry.active > 0 || entry.left
                .is_some_and(|left| now.saturating_duration_since(left) < SIGNED_OUT_GRACE));
            if endpoints.len() >= MAX_GRACE_ENTRIES {
                let oldest = endpoints.iter().filter(|(_, entry)| entry.active == 0)
                    .min_by_key(|(_, entry)| entry.left).map(|(endpoint, _)| *endpoint);
                if let Some(oldest) = oldest { endpoints.remove(&oldest); }
            }
        }
        let admission = endpoints.entry(endpoint).or_insert(Admission {
            active: 0,
            left: None,
        });
        admission.active += 1;
        admission.left = None;
    }

    pub(super) fn signed_out(&self, endpoint: EndpointId) {
        let mut endpoints = self.lock();
        let now = Instant::now();
        if let Some(admission) = endpoints.get_mut(&endpoint) {
            admission.active = admission.active.saturating_sub(1);
            if admission.active == 0 {
                admission.left = Some(now);
            }
        }
        if endpoints.len() > MAX_GRACE_ENTRIES {
            endpoints.retain(|_, admission| {
                admission.active > 0
                    || admission
                        .left
                        .is_some_and(|left| now.saturating_duration_since(left) < SIGNED_OUT_GRACE)
            });
        }
    }

    pub(super) fn admits(&self, endpoint: &EndpointId) -> bool {
        self.lock().get(endpoint).is_some_and(|admission| {
            admission.active > 0
                || admission
                    .left
                    .is_some_and(|left| left.elapsed() < SIGNED_OUT_GRACE)
        })
    }
}

#[derive(Debug)]
pub(super) struct RelayAccess {
    counts: Mutex<AdmissionCounts<EndpointId, ConnectionId>>,
    admissions: Arc<RelayAdmissions>,
    max_total: usize,
    max_per_endpoint: usize,
    ping_schedule: ClientPingSchedule,
}

impl RelayAccess {
    pub(super) fn new(
        admissions: Arc<RelayAdmissions>,
        max_total: usize,
        max_per_endpoint: usize,
        ping_schedule: ClientPingSchedule,
    ) -> Self {
        Self {
            counts: Mutex::new(AdmissionCounts::default()),
            admissions,
            max_total,
            max_per_endpoint,
            ping_schedule,
        }
    }

    fn with_counts<R>(
        &self,
        callback: impl FnOnce(&mut AdmissionCounts<EndpointId, ConnectionId>) -> R,
    ) -> R {
        let mut counts = match self.counts.lock() {
            Ok(counts) => counts,
            Err(poisoned) => poisoned.into_inner(),
        };
        callback(&mut counts)
    }
}

impl AccessControl for RelayAccess {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        // Iroh authenticates this EndpointId during its challenge/proof handshake before
        // invoking the access hook, so an outside client cannot choose another key's bucket.
        let endpoint = request.endpoint_id();
        if !self.admissions.admits(&endpoint) {
            return Access::Deny {
                reason: Some(RELAY_SIGNALING_DENIAL.to_string()),
            };
        }
        let admitted = self.with_counts(|counts| {
            counts.try_admit(
                endpoint,
                request.connection_id(),
                self.max_total,
                self.max_per_endpoint,
            )
        });
        if admitted {
            Access::Allow
        } else {
            Access::Deny {
                reason: Some(RELAY_CAPACITY_DENIAL.to_string()),
            }
        }
    }

    fn on_disconnect(&self, _endpoint_id: EndpointId, connection_id: ConnectionId) {
        self.with_counts(|counts| counts.release(&connection_id));
    }

    fn ping_schedule(&self, _endpoint_id: EndpointId) -> ClientPingSchedule {
        self.ping_schedule
    }
}

#[derive(Debug)]
pub(super) struct AdmissionCounts<K, C> {
    by_endpoint: HashMap<K, usize>,
    admitted: HashMap<C, K>,
}

impl<K, C> Default for AdmissionCounts<K, C> {
    fn default() -> Self {
        Self {
            by_endpoint: HashMap::new(),
            admitted: HashMap::new(),
        }
    }
}

impl<K: Clone + Eq + Hash, C: Eq + Hash> AdmissionCounts<K, C> {
    pub(super) fn try_admit(
        &mut self,
        endpoint_id: K,
        connection_id: C,
        max_total: usize,
        max_per_endpoint: usize,
    ) -> bool {
        let endpoint_count = self.by_endpoint.get(&endpoint_id).copied().unwrap_or(0);
        if self.admitted.len() >= max_total
            || endpoint_count >= max_per_endpoint
            || self.admitted.contains_key(&connection_id)
        {
            return false;
        }
        self.by_endpoint
            .insert(endpoint_id.clone(), endpoint_count + 1);
        self.admitted.insert(connection_id, endpoint_id);
        true
    }

    pub(super) fn release(&mut self, connection_id: &C) {
        let Some(endpoint_id) = self.admitted.remove(connection_id) else {
            return;
        };
        let Some(endpoint_count) = self.by_endpoint.get_mut(&endpoint_id) else {
            return;
        };
        *endpoint_count = endpoint_count.saturating_sub(1);
        if *endpoint_count == 0 {
            self.by_endpoint.remove(&endpoint_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_admission_enforces_per_endpoint_cap_and_reuses_capacity() {
        let mut counts = AdmissionCounts::default();
        assert!(counts.try_admit(7_u8, 10_u8, 8, 2));
        assert!(counts.try_admit(7_u8, 11_u8, 8, 2));
        assert!(!counts.try_admit(7_u8, 12_u8, 8, 2));

        counts.release(&10);
        assert!(counts.try_admit(7_u8, 12_u8, 8, 2));
        counts.release(&10);
        counts.release(&11);
        counts.release(&12);

        assert!(counts.admitted.is_empty());
        assert!(!counts.by_endpoint.contains_key(&7));
    }

    #[test]
    fn relay_admission_enforces_global_cap_and_reuses_capacity() {
        let mut counts = AdmissionCounts::default();
        assert!(counts.try_admit(1_u8, 10_u8, 3, 2));
        assert!(counts.try_admit(1_u8, 11_u8, 3, 2));
        assert!(counts.try_admit(2_u8, 12_u8, 3, 2));
        assert!(!counts.try_admit(3_u8, 13_u8, 3, 2));

        counts.release(&10);
        assert!(counts.try_admit(3_u8, 13_u8, 3, 2));
        assert_eq!(counts.admitted.len(), 3);
    }

    #[test]
    fn review_task_relay_admits_only_signaling_registered_endpoints() {
        let admissions = RelayAdmissions::default();
        let endpoint = iroh_base::SecretKey::from_bytes(&[9; 32]).public();
        let stranger = iroh_base::SecretKey::from_bytes(&[10; 32]).public();
        assert!(!admissions.admits(&endpoint));
        admissions.signed_in(endpoint);
        admissions.signed_in(endpoint);
        assert!(admissions.admits(&endpoint));
        assert!(!admissions.admits(&stranger));
        admissions.signed_out(endpoint);
        admissions.signed_out(endpoint);
        // A reconnecting endpoint keeps its relay during the grace period.
        assert!(admissions.admits(&endpoint));
    }
}
