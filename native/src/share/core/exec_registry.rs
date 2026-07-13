use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use super::exec_types::{
    ExecAuthorization, ExecId, ExecJobView, ExecLifecycleState, ExecPrincipal, ExecStart,
    ExecTerminal, ExecTerminalKind,
};

#[path = "exec_registry_view.rs"]
mod registry_view;
use registry_view::{terminal_view, view};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecCancelReason {
    User,
    Timeout,
    Revoked,
    Disconnected,
    WorkerStopping,
}

#[derive(Clone, Debug)]
pub(crate) struct ExecCancellation(Arc<Mutex<Option<ExecCancelReason>>>);

impl ExecCancellation {
    pub(crate) fn reason(&self) -> Option<ExecCancelReason> {
        self.0.lock().ok().and_then(|reason| *reason)
    }

    fn cancel(&self, reason: ExecCancelReason) {
        if let Ok(mut current) = self.0.lock() {
            current.get_or_insert(reason);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExecAuthorizationLease {
    pub(crate) principal: ExecPrincipal,
    pub(crate) policy_revision: u64,
    pub(crate) authorization_epoch: u64,
    pub(crate) session_id: String,
    pub(crate) exec_id: ExecId,
    pub(crate) command_digest: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ExecReservation {
    pub(crate) lease: ExecAuthorizationLease,
    pub(crate) cancellation: ExecCancellation,
}

#[derive(Clone, Debug)]
pub(crate) enum ExecAdmission {
    Prepared(ExecReservation),
    AlreadyRunning(ExecJobView),
    CachedTerminal(ExecJobView),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExecRegistryError {
    InvalidAuthorization,
    InvalidStart,
    NotAuthorized,
    StaleAuthorization,
    DuplicateMismatch,
    GlobalLimit,
    PrincipalLimit,
    UnknownExecution,
    NotPreparing,
    ContainmentNotConfirmed,
    TerminalReasonMismatch,
    LaunchFailed(String),
}

impl std::fmt::Display for ExecRegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ExecRegistryLimits {
    pub(crate) global_active: usize,
    pub(crate) principal_active: usize,
    pub(crate) history: usize,
    pub(crate) terminal_cache: usize,
}

impl Default for ExecRegistryLimits {
    fn default() -> Self {
        Self {
            global_active: 8,
            principal_active: 2,
            history: 128,
            terminal_cache: 128,
        }
    }
}

pub(crate) struct ExecRegistry {
    state: Mutex<RegistryState>,
    limits: ExecRegistryLimits,
}

#[derive(Default)]
struct RegistryState {
    authorization_epoch: u64,
    policies: HashMap<PrincipalIdentity, (u64, bool)>,
    active: HashMap<ExecId, ActiveJob>,
    history: VecDeque<ExecJobView>,
    terminal: HashMap<ExecId, TerminalEntry>,
    terminal_order: VecDeque<ExecId>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct PrincipalIdentity(String, String, String, String, String, String);

impl From<&ExecPrincipal> for PrincipalIdentity {
    fn from(value: &ExecPrincipal) -> Self {
        Self(
            value.relation_kind.clone(),
            value.relation_id.clone(),
            value.device_id.clone(),
            value.public_key.clone(),
            value.fingerprint.clone(),
            value.node_id.clone(),
        )
    }
}

struct ActiveJob {
    lease: ExecAuthorizationLease,
    program: String,
    state: ExecLifecycleState,
    started_at: Option<i64>,
    cancellation: ExecCancellation,
}

struct TerminalEntry {
    identity: PrincipalIdentity,
    digest: String,
    view: ExecJobView,
}

impl ExecRegistry {
    pub(crate) fn new(limits: ExecRegistryLimits) -> Self {
        Self {
            state: Mutex::new(RegistryState::default()),
            limits,
        }
    }

    pub(crate) fn apply_authorization(
        &self,
        principal: &ExecPrincipal,
        revision: u64,
        epoch: u64,
        enabled: bool,
    ) -> Result<(), ExecRegistryError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if epoch < state.authorization_epoch {
            return Err(ExecRegistryError::StaleAuthorization);
        }
        let epoch_advanced = epoch > state.authorization_epoch;
        if epoch_advanced {
            state.authorization_epoch = epoch;
            cancel_matching(&mut state, |_| true, ExecCancelReason::Revoked);
        }
        let identity = PrincipalIdentity::from(principal);
        if let Some((current_revision, current_enabled)) = state.policies.get(&identity) {
            if revision < *current_revision
                || (revision == *current_revision
                    && enabled
                    && !*current_enabled
                    && !epoch_advanced)
            {
                return Err(ExecRegistryError::StaleAuthorization);
            }
        }
        state.policies.insert(identity.clone(), (revision, enabled));
        if !enabled {
            cancel_matching(
                &mut state,
                |job| PrincipalIdentity::from(&job.lease.principal) == identity,
                ExecCancelReason::Revoked,
            );
        }
        Ok(())
    }

    pub(crate) fn prepare(
        &self,
        principal: ExecPrincipal,
        authorization: ExecAuthorization,
        start: &ExecStart,
        now: i64,
    ) -> Result<ExecAdmission, ExecRegistryError> {
        if authorization.session_id.trim().is_empty() {
            return Err(ExecRegistryError::InvalidAuthorization);
        }
        let digest = start
            .digest()
            .map_err(|_| ExecRegistryError::InvalidStart)?;
        let identity = PrincipalIdentity::from(&principal);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let Some(policy) = state.policies.get(&identity) else {
            return Err(ExecRegistryError::NotAuthorized);
        };
        if !policy.1 {
            return Err(ExecRegistryError::NotAuthorized);
        }
        if policy.0 != authorization.policy_revision
            || state.authorization_epoch != authorization.authorization_epoch
        {
            return Err(ExecRegistryError::StaleAuthorization);
        }
        if let Some(job) = state.active.get(&start.exec_id) {
            if PrincipalIdentity::from(&job.lease.principal) != identity
                || job.lease.command_digest != digest
            {
                return Err(ExecRegistryError::DuplicateMismatch);
            }
            return Ok(ExecAdmission::AlreadyRunning(view(job)));
        }
        if let Some(entry) = state.terminal.get(&start.exec_id) {
            if entry.identity != identity || entry.digest != digest {
                return Err(ExecRegistryError::DuplicateMismatch);
            }
            return Ok(ExecAdmission::CachedTerminal(entry.view.clone()));
        }
        if state.active.len() >= self.limits.global_active {
            return Err(ExecRegistryError::GlobalLimit);
        }
        let peer_active = state
            .active
            .values()
            .filter(|job| PrincipalIdentity::from(&job.lease.principal) == identity)
            .count();
        if peer_active >= self.limits.principal_active {
            return Err(ExecRegistryError::PrincipalLimit);
        }
        let cancellation = ExecCancellation(Arc::new(Mutex::new(None)));
        let lease = ExecAuthorizationLease {
            principal,
            policy_revision: authorization.policy_revision,
            authorization_epoch: authorization.authorization_epoch,
            session_id: authorization.session_id,
            exec_id: start.exec_id.clone(),
            command_digest: digest,
        };
        state.active.insert(
            start.exec_id.clone(),
            ActiveJob {
                lease: lease.clone(),
                program: start.display_program().to_string(),
                state: ExecLifecycleState::Starting,
                started_at: Some(now),
                cancellation: cancellation.clone(),
            },
        );
        Ok(ExecAdmission::Prepared(ExecReservation {
            lease,
            cancellation,
        }))
    }

    pub(crate) fn commit_start<F>(
        &self,
        lease: &ExecAuthorizationLease,
        release_launch_barrier: F,
    ) -> Result<ExecCancellation, ExecRegistryError>
    where
        F: FnOnce() -> Result<(), String>,
    {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let identity = PrincipalIdentity::from(&lease.principal);
        let Some(policy) = state.policies.get(&identity) else {
            return stale_job(&mut state, lease);
        };
        if !policy.1
            || policy.0 != lease.policy_revision
            || state.authorization_epoch != lease.authorization_epoch
        {
            return stale_job(&mut state, lease);
        }
        let Some(job) = state.active.get_mut(&lease.exec_id) else {
            return Err(ExecRegistryError::UnknownExecution);
        };
        if job.lease != *lease || job.state != ExecLifecycleState::Starting {
            return Err(ExecRegistryError::NotPreparing);
        }
        if job.cancellation.reason().is_some() {
            return Err(ExecRegistryError::NotPreparing);
        }
        if let Err(error) = release_launch_barrier() {
            job.state = ExecLifecycleState::Cancelling;
            return Err(ExecRegistryError::LaunchFailed(error));
        }
        job.state = ExecLifecycleState::Running;
        Ok(job.cancellation.clone())
    }

    pub(crate) fn fail_preparation(
        &self,
        lease: &ExecAuthorizationLease,
        message: String,
        finished_at: i64,
    ) -> Result<ExecJobView, ExecRegistryError> {
        self.record_terminal(
            lease,
            ExecTerminal {
                exec_id: lease.exec_id.clone(),
                kind: ExecTerminalKind::Failed,
                exit_code: None,
                signal: None,
                message: Some(message),
                stdout_bytes: 0,
                stderr_bytes: 0,
                output_truncated: false,
            },
            true,
            finished_at,
        )
    }

    pub(crate) fn cancel(&self, exec_id: &ExecId, reason: ExecCancelReason) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let Some(job) = state.active.get_mut(exec_id) else {
            return false;
        };
        job.cancellation.cancel(reason);
        job.state = ExecLifecycleState::Cancelling;
        true
    }

    pub(crate) fn cancel_exact(
        &self,
        exec_id: &ExecId,
        peer_device_id: &str,
        reason: ExecCancelReason,
    ) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let Some(job) = state.active.get_mut(exec_id) else {
            return false;
        };
        if job.lease.principal.device_id != peer_device_id {
            return false;
        }
        job.cancellation.cancel(reason);
        job.state = ExecLifecycleState::Cancelling;
        true
    }

    pub(crate) fn cancel_all(&self, reason: ExecCancelReason) -> usize {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        cancel_matching(&mut state, |_| true, reason)
    }

    pub(crate) fn record_terminal(
        &self,
        lease: &ExecAuthorizationLease,
        terminal: ExecTerminal,
        containment_confirmed_empty: bool,
        finished_at: i64,
    ) -> Result<ExecJobView, ExecRegistryError> {
        if !containment_confirmed_empty {
            return Err(ExecRegistryError::ContainmentNotConfirmed);
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(cached) = state.terminal.get(&lease.exec_id) {
            return if cached.digest == lease.command_digest
                && cached.identity == PrincipalIdentity::from(&lease.principal)
            {
                Ok(cached.view.clone())
            } else {
                Err(ExecRegistryError::DuplicateMismatch)
            };
        }
        let Some(job) = state.active.remove(&lease.exec_id) else {
            return Err(ExecRegistryError::UnknownExecution);
        };
        if job.lease != *lease || terminal.exec_id != lease.exec_id {
            state.active.insert(job.lease.exec_id.clone(), job);
            return Err(ExecRegistryError::DuplicateMismatch);
        }
        if !terminal_matches(job.cancellation.reason(), &terminal.kind) {
            state.active.insert(job.lease.exec_id.clone(), job);
            return Err(ExecRegistryError::TerminalReasonMismatch);
        }
        let view = terminal_view(&job, terminal, finished_at);
        if self.limits.history != 0 {
            state.history.push_back(view.clone());
            if state.history.len() > self.limits.history {
                state.history.pop_front();
            }
        }
        state.terminal.insert(
            lease.exec_id.clone(),
            TerminalEntry {
                identity: PrincipalIdentity::from(&lease.principal),
                digest: lease.command_digest.clone(),
                view: view.clone(),
            },
        );
        state.terminal_order.push_back(lease.exec_id.clone());
        while state.terminal_order.len() > self.limits.terminal_cache {
            if let Some(expired) = state.terminal_order.pop_front() {
                state.terminal.remove(&expired);
            }
        }
        Ok(view)
    }

    pub(crate) fn active_views(&self) -> Vec<ExecJobView> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.active.values().map(view).collect()
    }

    pub(crate) fn redacted_history(&self) -> Vec<ExecJobView> {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state
            .history
            .iter()
            .cloned()
            .map(|mut view| {
                if let Some(terminal) = &mut view.terminal {
                    terminal.message = None;
                }
                view
            })
            .collect()
    }
}

fn stale_job<T>(
    state: &mut RegistryState,
    lease: &ExecAuthorizationLease,
) -> Result<T, ExecRegistryError> {
    if let Some(job) = state.active.get_mut(&lease.exec_id) {
        job.cancellation.cancel(ExecCancelReason::Revoked);
        job.state = ExecLifecycleState::Cancelling;
    }
    Err(ExecRegistryError::StaleAuthorization)
}

fn cancel_matching(
    state: &mut RegistryState,
    mut matches: impl FnMut(&ActiveJob) -> bool,
    reason: ExecCancelReason,
) -> usize {
    let mut cancelled = 0;
    for job in state.active.values_mut().filter(|job| matches(job)) {
        job.cancellation.cancel(reason);
        job.state = ExecLifecycleState::Cancelling;
        cancelled += 1;
    }
    cancelled
}

fn terminal_matches(reason: Option<ExecCancelReason>, terminal: &ExecTerminalKind) -> bool {
    match reason {
        None => matches!(
            terminal,
            ExecTerminalKind::Exited | ExecTerminalKind::Failed
        ),
        Some(ExecCancelReason::User | ExecCancelReason::WorkerStopping) => {
            *terminal == ExecTerminalKind::Cancelled
        }
        Some(ExecCancelReason::Timeout) => *terminal == ExecTerminalKind::TimedOut,
        Some(ExecCancelReason::Revoked) => *terminal == ExecTerminalKind::Revoked,
        Some(ExecCancelReason::Disconnected) => *terminal == ExecTerminalKind::Disconnected,
    }
}

#[cfg(test)]
#[path = "exec_registry_tests.rs"]
mod tests;
