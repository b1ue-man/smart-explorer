// Smart Explorer P28b/V3 patch (2026-10-25); see vendor/SMART_EXPLORER_ROUTE_POLICY.md.
//! Fresh, finite policy proofs and bounded replacement of redundant candidates.

use super::{
    ApplicationPathPolicyError, Connection, PathId, SpaceKind,
    timer::{PathTimer, Timer},
};
use crate::{Duration, Instant, VarInt};

impl Connection {
    pub(super) fn application_validation_reference_pto(&self) -> Duration {
        let admitted_pto = self.application_path_policy.admitted
            .filter(|(id, _)| self.paths.contains_key(id))
            .map(|(id, _)| self.pto(SpaceKind::Data, id));
        self.application_initial_pto()
            .max(admitted_pto.unwrap_or_else(|| self.application_best_pto()))
    }

    pub(super) fn start_application_path_validation(&mut self, now: Instant, reference_pto: Duration) {
        let ids: Vec<_> = self.paths.keys().copied()
            .filter(|id| !self.abandoned_paths.contains(id)).collect();
        for id in ids {
            self.start_application_path_validation_on(now, id, reference_pto);
        }
    }

    fn start_application_path_validation_on(&mut self, now: Instant, id: PathId, reference_pto: Duration) {
        let Some(path) = self.paths.get(&id) else { return; };
        let previous_pto = path.prev.as_ref().map_or(Duration::ZERO, |(_, prev)| {
            prev.rtt.pto_base() + self.ack_frequency.max_ack_delay_for_pto()
        });
        let wait = reference_pto.max(self.pto(SpaceKind::Data, id))
            .max(previous_pto).saturating_mul(3);
        self.application_path_policy.pending_validation.insert(id);
        let Some(path) = self.paths.get_mut(&id) else { return; };
        // A response from a previous app generation/attempt must neither prove
        // this attempt nor cancel its pending challenge. Keep QUIC validation,
        // RTT/congestion state and the previous migration incarnation intact.
        path.data.reset_on_path_challenges();
        self.timers.stop(Timer::PerPath(id, PathTimer::PathChallengeLost), self.qlog.with_time(now));
        self.timers.stop(Timer::PerPath(id, PathTimer::PathValidationFailed), self.qlog.with_time(now));
        if let Some(deadline) = now.checked_add(wait) {
            path.data.pending_challenge = true;
            self.timers.set(Timer::PerPath(id, PathTimer::PathValidationFailed), deadline, self.qlog.with_time(now));
        }
        // An unrepresentable deadline leaves a blocked, inactive attempt; it
        // cannot accidentally create an unbounded challenge-retry loop.
    }

    pub(super) fn retry_application_path_validation(&mut self, now: Instant, id: PathId) {
        if !self.config.application_path_policy
            || !self.application_path_policy.pending_validation.contains(&id)
            || self.abandoned_paths.contains(&id)
            || !self.paths.get(&id).is_some_and(|path| {
                !path.data.is_validating_path()
                    && !path.prev.as_ref().is_some_and(|(_, prev)| prev.is_validating_path())
            })
            || self.timers.get(Timer::PerPath(id, PathTimer::PathValidationFailed)).is_some()
        {
            return;
        }
        // A subsequent open/NAT attempt or the existing per-path keepalive is
        // the retry trigger. ACKs never grant data; active rounds keep their deadline.
        let reference_pto = self.application_validation_reference_pto();
        self.start_application_path_validation_on(now, id, reference_pto);
    }

    pub(super) fn application_path_validation_failed(&mut self, id: PathId) {
        if !self.config.application_path_policy
            || !self.application_path_policy.pending_validation.contains(&id)
        {
            return;
        }
        // The upstream timeout may just have restored prev. Its pre-attempt
        // tokens cannot satisfy the additional app proof after that rollback.
        if let Some(path) = self.paths.get_mut(&id) {
            path.data.reset_on_path_challenges();
        }
    }

    fn application_path_usable_for_retirement(&self, id: PathId) -> bool {
        self.state.is_established()
            && !self.abandoned_paths.contains(&id)
            && !self.application_path_policy.pending_validation.contains(&id)
            && self.remote_cids.contains_key(&id)
            && self.paths.get(&id).is_some_and(|path| {
                path.data.validated && !path.data.is_validating_path()
                    && !path.prev.as_ref().is_some_and(|(_, prev)| prev.is_validating_path())
            })
    }

    pub(super) fn application_path_retirement_allowed(&self, id: PathId) -> bool {
        if !self.config.application_path_policy || !self.side().is_client()
            || self.application_path_policy.requires_new_generation
            || self.abandoned_paths.contains(&id)
            || self.application_path_policy.admitted.is_some_and(|(admitted, _)| admitted == id)
            || !self.paths.get(&id).is_some_and(|path| {
                !path.data.is_validating_path()
                    && !path.prev.as_ref().is_some_and(|(_, prev)| prev.is_validating_path())
            })
        {
            return false;
        }
        // Mirror open_path's actual negotiated PathId limit, including already
        // abandoned IDs. A concurrent MAX_PATH_ID can make retirement needless.
        let next_id = self.abandoned_paths.max().max(self.paths.keys().last().copied())
            .unwrap_or(PathId::ZERO).saturating_add(1u8);
        if !self.max_path_id().is_some_and(|limit| next_id > limit) {
            return false;
        }
        // At most one retirement can await draining. This avoids evicting the
        // entire candidate set while MAX_PATH_ID/CIDs for one slot are pending.
        if self.paths.keys().any(|path| self.abandoned_paths.contains(path)) {
            return false;
        }
        self.paths.keys().any(|&other| other != id && self.application_path_usable_for_retirement(other))
    }

    pub(super) fn retire_redundant_application_path(
        &mut self,
        now: Instant,
        id: PathId,
    ) -> Result<(), ApplicationPathPolicyError> {
        if !self.application_path_retirement_allowed(id) {
            return Err(ApplicationPathPolicyError::ProtectedPath);
        }
        self.close_path(now, id, VarInt::from_u32(0))
            .map_err(|_| ApplicationPathPolicyError::ProtectedPath)
    }
}
