//! Fail-closed, single-use authorization. Entropy and OS facts enter as values.
use super::model::{
    Challenge, DesktopKind, Operation, Policy, SessionBinding, SignedRequest, VERSION,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Denial {
    pub state: &'static str,
    pub reason: &'static str,
}
impl std::fmt::Display for Denial {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(self.reason)
    }
}
fn deny(state: &'static str, reason: &'static str) -> Denial {
    Denial { state, reason }
}

pub(crate) struct Authority {
    current: Challenge,
    peer_id: String,
    target_user: String,
    consumed: bool,
}

impl Authority {
    pub(crate) fn new(
        policy: &Policy,
        boot: String,
        binding: SessionBinding,
        peer_id: String,
        target_user: String,
        nonce: String,
        sequence: u64,
        now: u64,
    ) -> Self {
        Self {
            current: Challenge {
                format: VERSION,
                machine_id: policy.machine_id.clone(),
                machine_public_key: policy.machine_public_key.clone(),
                boot_generation: boot,
                binding,
                policy_revision: policy.revision,
                nonce,
                sequence,
                expires_at: now.saturating_add(30),
            },
            peer_id,
            target_user,
            consumed: false,
        }
    }

    pub(crate) fn challenge(&self) -> &Challenge {
        &self.current
    }

    pub(crate) fn authorize(
        &mut self,
        request: &SignedRequest,
        policy: &Policy,
        binding: &SessionBinding,
        now: u64,
    ) -> Result<(), Denial> {
        if self.consumed
            || request.challenge != self.current
            || now > self.current.expires_at
            || policy.revision != self.current.policy_revision
            || binding != &self.current.binding
            || policy.machine_id != self.current.machine_id
            || policy.machine_public_key != self.current.machine_public_key
            || request.peer_id != self.peer_id
            || request.target_user != self.target_user
        {
            return Err(deny(
                "session_changed",
                "stale or mismatched login challenge",
            ));
        }
        request
            .operation
            .validate()
            .map_err(|reason| deny("invalid_request", reason))?;
        let grant = policy
            .grants
            .iter()
            .find(|grant| grant.peer_id == request.peer_id)
            .ok_or_else(|| deny("right_missing", "peer has no administrative login grant"))?;
        if now < grant.not_before
            || now >= grant.expires_at
            || !grant.target_users.contains(&request.target_user)
            || binding
                .user_id
                .as_ref()
                .is_some_and(|id| id != &request.target_user)
        {
            return Err(deny(
                "right_missing",
                "target user or grant lifetime is not authorized",
            ));
        }
        let rights = &grant.rights;
        if !rights.login || !rights.screen_view {
            return Err(deny(
                "right_missing",
                "login and screen view rights are required",
            ));
        }
        match binding.desktop {
            DesktopKind::BeforeLogin | DesktopKind::X11Greeter if !rights.before_login => {
                return Err(deny("right_missing", "before-login right is required"))
            }
            DesktopKind::Locked if !rights.locked => {
                return Err(deny("right_missing", "locked-desktop right is required"))
            }
            DesktopKind::Uac if !rights.uac => {
                return Err(deny("right_missing", "UAC-desktop right is required"))
            }
            DesktopKind::WaylandGreeter => {
                return Err(deny(
                    "platform_unsupported",
                    "Wayland greeter capture and input are unavailable",
                ))
            }
            DesktopKind::UserSession => {
                return Err(deny(
                    "user_session_active",
                    "use the existing user worker for an unlocked user session",
                ))
            }
            DesktopKind::Unavailable => {
                return Err(deny(
                    "session_unavailable",
                    "no eligible active login desktop",
                ))
            }
            _ => {}
        }
        if matches!(
            request.operation,
            Operation::Input { .. } | Operation::SecureAttention | Operation::ReleaseInputs
        ) && !rights.screen_control
        {
            return Err(deny("right_missing", "screen control right is required"));
        }
        if matches!(request.operation, Operation::SecureAttention) && !rights.sas {
            return Err(deny("right_missing", "secure-attention right is required"));
        }
        let payload = request
            .signing_bytes()
            .map_err(|_| deny("invalid_request", "invalid signed login payload"))?;
        if !crate::share::verify_iroh_signature(&grant.public_key, &payload, &request.signature) {
            return Err(deny("invalid_request", "invalid peer login signature"));
        }
        // Consume before any privileged effect, including an effect that subsequently fails.
        self.consumed = true;
        Ok(())
    }
}
