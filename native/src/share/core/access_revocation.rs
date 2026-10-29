//! Authority of one concrete AccessV2 operation. Hosts hold a lease beside
//! the live channel and recheck under the auth lock before admitting work,
//! on policy/session changes and at its deadline. A summary of relationships
//! can never substitute for this exact relationship binding.
use crate::share::access_authorize::{principal_grant, request_for};
use crate::share::access_constraints::{earlier, AccessRequest, SessionId, UserId};
use crate::share::access_grant::{AccountProfile, CapabilityDecision};
use crate::share::relation_proof::ProvenPrincipal;
use crate::share::types::ShareAuthState;

/// Created by the approval ticket only after the human answered. The value
/// is consumed by admission and cannot be cloned for another session.
pub(crate) struct AccessConfirmation {
    principal: ProvenPrincipal,
    request: AccessRequest,
    revision: u64,
}
pub(crate) fn confirmation(
    principal: ProvenPrincipal,
    request: AccessRequest,
    revision: u64,
) -> AccessConfirmation {
    AccessConfirmation {
        principal,
        request,
        revision,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccessRevocation {
    Unavailable,
    HostStopped,
    SessionChanged,
    UserChanged,
    RelationEnded,
    PermissionWithdrawn,
    AccountChanged,
    Expired,
    EarlierDeadline,
    DecisionNarrowed,
}

/// Not serializable: a stored or remotely supplied lease is no authority.
pub(crate) struct AccessLease {
    principal: ProvenPrincipal,
    request: AccessRequest,
    account: AccountProfile,
    decision: CapabilityDecision,
    end: Option<i64>,
}
impl AccessLease {
    /// Call under the same auth lock as registry insertion. Ask requires a
    /// single-use confirmation bound to this request and exact revision.
    pub(crate) fn admit(
        state: &ShareAuthState,
        principal: &ProvenPrincipal,
        request: AccessRequest,
        confirmed: Option<AccessConfirmation>,
        now: i64,
    ) -> Result<Self, AccessRevocation> {
        let grant = principal_grant(state, principal).ok_or(AccessRevocation::RelationEnded)?;
        let decision = request_for(state, principal, &request, now).decision;
        let allowed = match decision {
            CapabilityDecision::Allow => true,
            CapabilityDecision::Ask => confirmed.is_some_and(|confirmation| {
                confirmation.principal == *principal
                    && confirmation.request == request
                    && confirmation.revision == grant.policy_revision
            }),
            CapabilityDecision::Off => false,
        };
        if !allowed {
            return Err(AccessRevocation::PermissionWithdrawn);
        }
        let access = grant.access_grant();
        let end = lease_end(&access, &request, now);
        Ok(Self {
            principal: principal.clone(),
            request,
            account: access.account,
            decision,
            end,
        })
    }
    pub(crate) fn principal(&self) -> &ProvenPrincipal {
        &self.principal
    }
    pub(crate) fn request(&self) -> &AccessRequest {
        &self.request
    }
    pub(crate) fn deadline(&self) -> Option<i64> {
        self.end
    }
    /// Only a channel no longer covered by its own concrete grant closes.
    /// Removing an unrelated app, user, port or host leaves this lease alive.
    /// The caller owns cancellation and input release; this pure decision
    /// never records a mutating job as successfully completed.
    pub(crate) fn recheck(
        &mut self,
        state: &ShareAuthState,
        now: i64,
    ) -> Result<(), AccessRevocation> {
        let grant =
            principal_grant(state, &self.principal).ok_or(AccessRevocation::RelationEnded)?;
        let access = grant.access_grant();
        if access.expired_at(now) {
            return Err(AccessRevocation::Expired);
        }
        if self.request.capability().uses_account() && access.account != self.account {
            return Err(AccessRevocation::AccountChanged);
        }
        let decision = request_for(state, &self.principal, &self.request, now).decision;
        if decision == CapabilityDecision::Off {
            return Err(if self.end.is_some_and(|end| end <= now) {
                AccessRevocation::Expired
            } else {
                AccessRevocation::PermissionWithdrawn
            });
        }
        if decision.rank() < self.decision.rank() {
            return Err(AccessRevocation::DecisionNarrowed);
        }
        let end = lease_end(&access, &self.request, now);
        if earlier(end, self.end) {
            return Err(AccessRevocation::EarlierDeadline);
        }
        self.end = end;
        // A lease upgraded Ask -> Allow must observe a subsequent Allow -> Ask.
        self.decision = decision;
        Ok(())
    }
}
/// The end of a lease for `request`: the grant's own end and, unless
/// steering (screen control) includes the request regardless of its
/// constraints, the constraint's end. A steered lease past a constraint end
/// it does not depend on would otherwise stay alive with a deadline in the
/// past, and the expiry loop would wake without pause.
fn lease_end(
    access: &crate::share::access_grant::AccessGrant,
    request: &AccessRequest,
    now: i64,
) -> Option<i64> {
    let steered = crate::share::access_steering::steers(access, now)
        && crate::share::access_steering::includes_request(request);
    let constrained = (!steered)
        .then(|| access.constraints.end_for(request, now))
        .flatten();
    earliest(access.valid_until, constrained)
}
fn earliest(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (left, right) => left.or(right),
    }
}

/// Empty program lists mean unrestricted commands. Adding programs to an
/// already restricted list widens and must not kill its running jobs.
pub(crate) fn program_list_narrows(old: &[String], new: &[String]) -> bool {
    !new.is_empty() && (old.is_empty() || old.iter().any(|program| !new.contains(program)))
}

/// Shared live-channel boundary for app, login and TCP hosts. The same
/// registry is rechecked on every authoritative policy transition and expiry;
/// each revoked receiver wakes its owning host adapter for resource cleanup.
pub(crate) struct AccessLeaseRegistry {
    inner: std::sync::Mutex<LeaseEntries>,
    changed: tokio::sync::watch::Sender<()>,
}
impl Default for AccessLeaseRegistry {
    fn default() -> Self {
        let (changed, _) = tokio::sync::watch::channel(());
        Self {
            inner: Default::default(),
            changed,
        }
    }
}
#[derive(Default)]
struct LeaseEntries {
    closed: bool,
    next: u64,
    entries: std::collections::BTreeMap<
        u64,
        (
            AccessLease,
            tokio::sync::watch::Sender<Option<AccessRevocation>>,
        ),
    >,
}
impl AccessLeaseRegistry {
    /// Keep the auth lock held through this call and resource admission, so
    /// revocation cannot interleave between a check and registry insertion.
    pub(crate) fn admit(
        self: &std::sync::Arc<Self>,
        state: &ShareAuthState,
        principal: &ProvenPrincipal,
        request: AccessRequest,
        confirmed: Option<AccessConfirmation>,
        now: i64,
    ) -> Result<AccessHandle, AccessRevocation> {
        let lease = AccessLease::admit(state, principal, request, confirmed, now)?;
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| AccessRevocation::Unavailable)?;
        if inner.closed {
            return Err(AccessRevocation::HostStopped);
        }
        let id = inner
            .next
            .checked_add(1)
            .ok_or(AccessRevocation::Unavailable)?;
        inner.next = id;
        let (stop, revoked) = tokio::sync::watch::channel(None);
        inner.entries.insert(id, (lease, stop));
        drop(inner);
        self.changed.send_replace(());
        Ok(AccessHandle {
            id,
            registry: self.clone(),
            revoked,
        })
    }
    /// Recheck against the candidate under its auth lock before publishing
    /// that candidate. Only newly uncovered operations receive a stop signal.
    pub(crate) fn recheck(&self, state: &ShareAuthState, now: i64) -> usize {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut revoked = 0;
        let mut changed = false;
        for (lease, stop) in inner.entries.values_mut() {
            if stop.borrow().is_some() {
                continue;
            }
            let before = lease.deadline();
            if let Err(reason) = lease.recheck(state, now) {
                stop.send_replace(Some(reason));
                revoked += 1;
            }
            changed |= before != lease.deadline();
        }
        drop(inner);
        if changed || revoked != 0 {
            self.changed.send_replace(());
        }
        revoked
    }
    pub(crate) fn changes(&self) -> tokio::sync::watch::Receiver<()> {
        self.changed.subscribe()
    }
    /// Attach a resource to this registry's existing concrete lease. The caller
    /// keeps lifecycle and auth through the callback, as for initial admission.
    /// Ask confirmation was consumed by this same lease, never reconstructed.
    pub(crate) fn admit_existing<T>(
        self: &std::sync::Arc<Self>,
        state: &ShareAuthState,
        handle: &AccessHandle,
        now: i64,
        admit: impl FnOnce(&AccessHandle) -> Result<T, AccessRevocation>,
    ) -> Result<T, AccessRevocation> {
        if !std::sync::Arc::ptr_eq(self, &handle.registry) {
            return Err(AccessRevocation::Unavailable);
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| AccessRevocation::Unavailable)?;
        if inner.closed {
            return Err(AccessRevocation::HostStopped);
        }
        let (lease, stop) = inner
            .entries
            .get_mut(&handle.id)
            .ok_or(AccessRevocation::Unavailable)?;
        if let Some(reason) = *stop.borrow() {
            return Err(reason);
        }
        if let Err(reason) = lease.recheck(state, now) {
            stop.send_replace(Some(reason));
            drop(inner);
            self.changed.send_replace(());
            return Err(reason);
        }
        drop(inner);
        admit(handle)
    }
    pub(crate) fn is_closed(&self) -> bool {
        self.inner.lock().map_or(true, |inner| inner.closed)
    }
    /// Permanently deny admission and wake every owning adapter. Handles
    /// stay registered until their adapters finish releasing resources.
    pub(crate) fn close(&self, reason: AccessRevocation) -> usize {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.closed = true;
        let mut revoked = 0;
        for (_, stop) in inner.entries.values() {
            if stop.borrow().is_none() {
                stop.send_replace(Some(reason));
                revoked += 1;
            }
        }
        drop(inner);
        self.changed.send_replace(());
        revoked
    }
    /// Host adapters report the prior stable session/generation before
    /// using its replacement. Grant lists cannot prove an OS session exists.
    pub(crate) fn invalidate_session(&self, session: &SessionId) -> usize {
        self.invalidate_matching(AccessRevocation::SessionChanged, |request| match request {
            AccessRequest::AppLaunch(request) => &request.session == session,
            AccessRequest::LoginAccess(request) => &request.session == session,
            AccessRequest::TcpForward(_) => false,
        })
    }
    pub(crate) fn invalidate_user(&self, user: &UserId) -> usize {
        self.invalidate_matching(AccessRevocation::UserChanged, |request| match request {
            AccessRequest::AppLaunch(request) => &request.user == user,
            AccessRequest::LoginAccess(request) => &request.user == user,
            AccessRequest::TcpForward(_) => false,
        })
    }
    fn invalidate_matching(
        &self,
        reason: AccessRevocation,
        matches: impl Fn(&AccessRequest) -> bool,
    ) -> usize {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut revoked = 0;
        for (lease, stop) in inner.entries.values() {
            if stop.borrow().is_none() && matches(lease.request()) {
                stop.send_replace(Some(reason));
                revoked += 1;
            }
        }
        revoked
    }
    pub(crate) fn next_deadline(&self) -> Option<i64> {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner
            .entries
            .values()
            .filter(|(_, stop)| stop.borrow().is_none())
            .filter_map(|(lease, _)| lease.deadline())
            .min()
    }
}
/// Holding this handle registers the live operation; dropping it removes
/// that operation. A stop signal never substitutes for host cleanup itself.
pub(crate) struct AccessHandle {
    id: u64,
    registry: std::sync::Arc<AccessLeaseRegistry>,
    revoked: tokio::sync::watch::Receiver<Option<AccessRevocation>>,
}
impl AccessHandle {
    pub(crate) fn reason(&self) -> Option<AccessRevocation> {
        *self.revoked.borrow()
    }
    pub(crate) fn revoked(&self) -> tokio::sync::watch::Receiver<Option<AccessRevocation>> {
        self.revoked.clone()
    }
}
impl Drop for AccessHandle {
    fn drop(&mut self) {
        self.registry
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .remove(&self.id);
        self.registry.changed.send_replace(());
    }
}
