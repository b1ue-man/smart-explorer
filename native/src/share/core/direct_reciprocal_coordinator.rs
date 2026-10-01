use std::cell::Cell;
use std::collections::{hash_map::DefaultHasher, HashMap};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::direct_reciprocal_transport::DirectReciprocalTransportResult;
use super::identity::ShareIdentity;
use super::node::ShareIrohNode;
use super::types::{DirectAccessState, DirectContact, PeerEndpoint, ShareScope};
// A bounded crossbeam channel behaves like `std::sync::mpsc::sync_channel`
// (same `try_send`/`recv_timeout`) and lets the signal worker wait for
// completions together with its other events.
use crossbeam_channel::{bounded as sync_channel, Receiver, Sender as SyncSender};

const MAX_REPAIRS: usize = 256;
const COMPLETION_CAPACITY: usize = 64;

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct DirectRepairKey {
    local_generation: u64,
    contact_id: String,
    remote_node_id: String,
}

impl DirectRepairKey {
    pub(crate) fn new(
        local_generation: u64,
        contact_id: impl Into<String>,
        remote_node_id: impl Into<String>,
    ) -> Result<Self, DirectRepairCandidateError> {
        let contact_id = contact_id.into();
        let remote_node_id = remote_node_id.into();
        if contact_id.trim().is_empty() || remote_node_id.trim().is_empty() {
            return Err(DirectRepairCandidateError::MissingIdentityPin);
        }
        Ok(Self {
            local_generation,
            contact_id,
            remote_node_id,
        })
    }
}

impl fmt::Debug for DirectRepairKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectRepairKey")
            .field("local_generation", &self.local_generation)
            .field("contact_id", &"[REDACTED]")
            .field("remote_node_id", &"[REDACTED]")
            .finish()
    }
}

pub(crate) struct DirectRepairCandidate {
    key: DirectRepairKey,
    endpoint: PeerEndpoint,
    identity: ShareIdentity,
}

impl DirectRepairCandidate {
    /// Accepts only a current, accepted Direct snapshot. An empty legacy node
    /// pin is allowed only after the saved key/fingerprint pins match; the
    /// transport repeats the decisive TLS/node equality check.
    pub(crate) fn from_accepted_contact(
        local_generation: u64,
        contact: &DirectContact,
        endpoint: PeerEndpoint,
        identity: ShareIdentity,
    ) -> Result<Self, DirectRepairCandidateError> {
        if contact.access_state != DirectAccessState::Accepted {
            return Err(DirectRepairCandidateError::PolicyDenied);
        }
        match &endpoint.scope {
            ShareScope::Direct { contact_id } if contact_id == &contact.id => {}
            ShareScope::Direct { .. } => return Err(DirectRepairCandidateError::ContactMismatch),
            ShareScope::Room { .. } => return Err(DirectRepairCandidateError::NotDirect),
        }
        if endpoint.presence.kind != "direct"
            || endpoint.presence.relation_id != contact.lookup_id
            || endpoint.relation_secret.len() != 32
            || endpoint.presence.public_key.trim().is_empty()
            || contact.expected_fingerprint.trim().is_empty()
        {
            return Err(DirectRepairCandidateError::InvalidMaterial);
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| DirectRepairCandidateError::StalePresence)?
            .as_secs()
            .min(i64::MAX as u64) as i64;
        if !endpoint.presence.is_current_at(now) {
            return Err(DirectRepairCandidateError::StalePresence);
        }
        let saved_device = contact
            .remote_device_id
            .as_deref()
            .ok_or(DirectRepairCandidateError::MissingIdentityPin)?;
        if saved_device != endpoint.presence.device_id
            || !contact
                .expected_fingerprint
                .eq_ignore_ascii_case(&endpoint.presence.fingerprint)
        {
            return Err(DirectRepairCandidateError::IdentityConflict);
        }
        let mut key_pinned = false;
        for saved in [
            contact.remote_public_key.as_deref(),
            contact.accepted_public_key.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            key_pinned = true;
            if saved != endpoint.presence.public_key {
                return Err(DirectRepairCandidateError::IdentityConflict);
            }
        }
        if !key_pinned || endpoint.presence.node_id.trim().is_empty() {
            return Err(DirectRepairCandidateError::MissingIdentityPin);
        }
        let endpoint_node_conflicts = endpoint
            .expected_node_id
            .as_deref()
            .filter(|node| !node.is_empty())
            .is_some_and(|node| node != endpoint.presence.node_id);
        if (!contact.expected_node_id.is_empty()
            && contact.expected_node_id != endpoint.presence.node_id)
            || endpoint_node_conflicts
        {
            return Err(DirectRepairCandidateError::IdentityConflict);
        }
        if identity.device_id.trim().is_empty()
            || identity.direct_lookup_id.trim().is_empty()
            || identity.public_key.trim().is_empty()
            || identity.node_id.trim().is_empty()
        {
            return Err(DirectRepairCandidateError::InvalidLocalIdentity);
        }
        let key = DirectRepairKey::new(
            local_generation,
            contact.id.clone(),
            endpoint.presence.node_id.clone(),
        )?;
        Ok(Self {
            key,
            endpoint,
            identity,
        })
    }
}

impl fmt::Debug for DirectRepairCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectRepairCandidate")
            .field("key", &self.key)
            .field("endpoint", &"[REDACTED]")
            .field("identity", &"[REDACTED]")
            .finish()
    }
}

impl Drop for DirectRepairCandidate {
    fn drop(&mut self) {
        self.endpoint.relation_secret.fill(0);
        self.identity.direct_secret.fill(0);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectRepairCandidateError {
    PolicyDenied,
    NotDirect,
    ContactMismatch,
    MissingIdentityPin,
    IdentityConflict,
    InvalidMaterial,
    InvalidLocalIdentity,
    StalePresence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectRepairScheduleOutcome {
    Queued,
    Refreshed,
    Suppressed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectRepairScheduleError {
    StaleGeneration,
    Capacity,
    Stopped,
}

pub(crate) struct DirectRepairCompletionReceiver {
    receiver: Receiver<()>,
    closed: Cell<bool>,
}

impl DirectRepairCompletionReceiver {
    fn new(receiver: Receiver<()>) -> Self {
        Self {
            receiver,
            closed: Cell::new(false),
        }
    }

    pub(crate) fn drain(&self) -> bool {
        let mut completed = false;
        loop {
            match self.receiver.try_recv() {
                Ok(()) => completed = true,
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.closed.set(true);
                    break;
                }
            }
        }
        completed
    }

    /// The channel to wait on, until the repair worker has ended.
    pub(crate) fn receiver(&self) -> Option<&Receiver<()>> {
        (!self.closed.get()).then_some(&self.receiver)
    }
}

struct State {
    generation: u64,
    stopped: bool,
    next_epoch: u64,
    tasks: HashMap<DirectRepairKey, RepairTask>,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

pub(crate) struct DirectReciprocalCoordinator {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl DirectReciprocalCoordinator {
    pub(crate) fn start(
        node: Arc<ShareIrohNode>,
        current_generation: u64,
    ) -> std::io::Result<(Self, DirectRepairCompletionReceiver)> {
        let (completions, receiver) = sync_channel(COMPLETION_CAPACITY);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                generation: current_generation,
                stopped: false,
                next_epoch: 1,
                tasks: HashMap::with_capacity(MAX_REPAIRS),
            }),
            wake: Condvar::new(),
        });
        let worker = thread::Builder::new()
            .name("direct-reciprocal".into())
            .spawn({
                let shared = shared.clone();
                move || worker::run_worker(node, shared, completions)
            })?;
        Ok((
            Self {
                shared,
                worker: Some(worker),
            },
            DirectRepairCompletionReceiver::new(receiver),
        ))
    }

    pub(crate) fn schedule(
        &self,
        candidate: DirectRepairCandidate,
    ) -> Result<DirectRepairScheduleOutcome, DirectRepairScheduleError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| DirectRepairScheduleError::Stopped)?;
        if state.stopped {
            return Err(DirectRepairScheduleError::Stopped);
        }
        if candidate.key.local_generation != state.generation {
            return Err(DirectRepairScheduleError::StaleGeneration);
        }
        let key = candidate.key.clone();
        if let Some(task) = state.tasks.get_mut(&key) {
            if task.blocked {
                return Ok(DirectRepairScheduleOutcome::Suppressed);
            }
            task.candidate = Some(candidate);
            self.shared.wake.notify_one();
            return Ok(DirectRepairScheduleOutcome::Refreshed);
        }
        if state.tasks.len() >= MAX_REPAIRS {
            return Err(DirectRepairScheduleError::Capacity);
        }
        let epoch = state.next_epoch;
        state.next_epoch = state.next_epoch.wrapping_add(1).max(1);
        state.tasks.insert(key, RepairTask::ready(epoch, candidate));
        self.shared.wake.notify_one();
        Ok(DirectRepairScheduleOutcome::Queued)
    }

    /// The owner increments this for every canonical profile or local identity
    /// authorization change. It clears terminal policy/conflict suppression.
    pub(crate) fn set_current_generation(&self, generation: u64) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.generation = generation;
            state
                .tasks
                .retain(|key, _| key.local_generation == generation);
            self.shared.wake.notify_one();
        }
    }

    pub(crate) fn request_stop(&self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.stopped = true;
            state.tasks.clear();
            self.shared.wake.notify_one();
        }
    }

    pub(crate) fn repair_in_flight(&self) -> bool {
        self.shared
            .state
            .lock()
            .map(|state| state.tasks.values().any(|task| task.running))
            .unwrap_or(true)
    }

    fn stop_worker(&mut self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.stopped = true;
            state.tasks.clear();
            self.shared.wake.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for DirectReciprocalCoordinator {
    fn drop(&mut self) {
        self.stop_worker();
    }
}

struct RepairTask {
    epoch: u64,
    candidate: Option<DirectRepairCandidate>,
    due: Option<Instant>,
    running: bool,
    blocked: bool,
    transient_attempt: u32,
    unsupported_attempt: u32,
}

impl RepairTask {
    fn ready(epoch: u64, candidate: DirectRepairCandidate) -> Self {
        Self {
            epoch,
            candidate: Some(candidate),
            due: Some(Instant::now()),
            running: false,
            blocked: false,
            transient_attempt: 0,
            unsupported_attempt: 0,
        }
    }
}

fn transient_delay(key: &DirectRepairKey, attempt: u32) -> Duration {
    match attempt {
        0 => Duration::from_secs(2),
        1 => Duration::from_secs(5),
        2 => Duration::from_secs(15),
        3 => Duration::from_secs(60),
        _ => jittered_delay(key, attempt, 300, 30),
    }
}

fn unsupported_delay(key: &DirectRepairKey, attempt: u32) -> Duration {
    jittered_delay(key, attempt, 30 * 60, 3 * 60)
}

fn jittered_delay(key: &DirectRepairKey, attempt: u32, base: u64, spread: u64) -> Duration {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    attempt.hash(&mut hasher);
    let width = spread.saturating_mul(2).saturating_add(1);
    let seconds = base
        .saturating_sub(spread)
        .saturating_add(hasher.finish() % width);
    Duration::from_secs(seconds)
}

#[cfg(test)]
#[path = "direct_reciprocal_coordinator_task_test_support.rs"]
mod task_test_support;

#[path = "direct_reciprocal_worker.rs"]
mod worker;
