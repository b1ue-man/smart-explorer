//! Durable one-time launch claims and admission-guarded dispatch.
use super::*;

impl<P: HostPlatform> AppHost<P> {
    pub(super) fn receipt_for(
        request: &LaunchRequest,
        state: LaunchState,
        reason: impl Into<String>,
    ) -> LaunchReceipt {
        let mut hash = Sha256::new();
        for part in [
            request.request_id.as_str(),
            request.target_device_id.as_str(),
            request.app_id.as_str(),
            request.user_id.as_str(),
            request.session_id.as_str(),
            &request.catalog_revision,
        ] {
            hash.update((part.len() as u64).to_le_bytes());
            hash.update(part.as_bytes());
        }
        LaunchReceipt {
            request: request.clone(),
            launch_id: format!("launch:{:x}", hash.finalize()),
            dispatch_id: String::new(),
            state,
            reason: reason.into(),
            pid: None,
            window: None,
        }
    }
    pub(super) fn new_dispatch(request: &LaunchRequest, reason: &str) -> io::Result<LaunchReceipt> {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes)
            .map_err(|_| io::Error::other("launch dispatch ID source unavailable"))?;
        let mut receipt = Self::receipt_for(request, LaunchState::OutcomeUnknown, reason);
        receipt.dispatch_id = bytes.iter().map(|b| format!("{b:02x}")).collect();
        Ok(receipt)
    }
    /// Repetition returns the durable receipt; changed contents under an ID are
    /// refused. `G` may be an E4 revocation/policy read guard. It is not serialized.
    pub(crate) fn launch<G>(
        &self,
        request: &LaunchRequest,
        authorize: impl FnOnce(&LaunchFacts) -> Result<LaunchAdmission<G>, LaunchDenial>,
    ) -> io::Result<LaunchReceipt> {
        self.launch_inner(request, false, authorize)
    }
    /// Local-UI continuation only. Main must retain the original authenticated
    /// peer context and run E4 again; never expose this flag on the remote wire.
    pub(crate) fn resume_after_local_consent<G>(
        &self,
        request: &LaunchRequest,
        authorize: impl FnOnce(&LaunchFacts) -> Result<LaunchAdmission<G>, LaunchDenial>,
    ) -> io::Result<LaunchReceipt> {
        self.launch_inner(request, true, authorize)
    }
    pub(super) fn launch_inner<G>(
        &self,
        request: &LaunchRequest,
        local_consent: bool,
        authorize: impl FnOnce(&LaunchFacts) -> Result<LaunchAdmission<G>, LaunchDenial>,
    ) -> io::Result<LaunchReceipt> {
        let _serial = self
            .serial
            .lock()
            .map_err(|_| io::Error::other("launch serialization poisoned"))?;
        if !self.journal.bind(request)? {
            return Ok(Self::receipt_for(
                request,
                LaunchState::Denied,
                "request_id_reused_with_different_request",
            ));
        }
        let previous = self.journal.read(&request.request_id)?;
        if previous.as_ref().is_some_and(|p| p.request != *request) {
            return Ok(Self::receipt_for(
                request,
                LaunchState::Denied,
                "request_id_reused_with_different_request",
            ));
        }
        let (catalog, native) = self.snapshot()?;
        let facts = match self.facts(request, &catalog) {
            Ok(facts) => facts,
            Err(reason) => {
                return self.denial(request, LaunchState::Denied, reason, previous.is_some())
            }
        };
        let admission = match authorize(&facts) {
            Ok(guard) => guard,
            Err(denial) => {
                let (state, reason) = denial.parts();
                return self.denial(request, state, reason, previous.is_some());
            }
        };
        if super::super::shared::platform::expired(admission.expires_at_unix_ms) {
            return self.denial(
                request,
                LaunchState::Denied,
                "app_launch_grant_expired".into(),
                previous.is_some(),
            );
        }
        let deadline = admission.expires_at_unix_ms;
        let valid = admission.valid;
        let _admission_guard = admission.guard;
        if let Some(receipt) = previous {
            if !local_consent || receipt.state != LaunchState::ConsentRequired {
                return Ok(receipt);
            }
            // Across processes only the creator of a new continuation claim may
            // dispatch. The original request itself remains permanently claimed.
            return self.continue_claimed(request, &native, deadline, valid);
        }
        let receipt = Self::new_dispatch(request, "admitted_os_result_not_yet_confirmed")?;
        if !self.journal.claim(&receipt)? {
            return self.claimed_receipt(request);
        }
        self.dispatch_claimed(request, &native, receipt, deadline, valid)
    }
    pub(super) fn claimed_receipt(&self, request: &LaunchRequest) -> io::Result<LaunchReceipt> {
        let receipt = self
            .journal
            .read(&request.request_id)?
            .ok_or_else(|| io::Error::other("launch claim disappeared"))?;
        if receipt.request != *request {
            return Ok(Self::receipt_for(
                request,
                LaunchState::Denied,
                "request_id_reused_with_different_request",
            ));
        }
        Ok(receipt)
    }
    pub(super) fn denial(
        &self,
        request: &LaunchRequest,
        state: LaunchState,
        reason: String,
        already_bound: bool,
    ) -> io::Result<LaunchReceipt> {
        let receipt = Self::receipt_for(request, state, reason);
        if !already_bound && !self.journal.claim(&receipt)? {
            return self.claimed_receipt(request);
        }
        if already_bound
            && self
                .journal
                .read(&request.request_id)?
                .is_some_and(|old| old.state == LaunchState::ConsentRequired)
            && matches!(
                receipt.state,
                LaunchState::Denied | LaunchState::Unsupported
            )
        {
            self.journal.update(&receipt)?;
        }
        Ok(receipt)
    }
    pub(super) fn continue_claimed(
        &self,
        request: &LaunchRequest,
        native: &PlatformCatalog<P::Target>,
        deadline: Option<i64>,
        valid: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) -> io::Result<LaunchReceipt> {
        // A second durable ID prevents two local continuations racing. It is
        // internal and cannot collide with the public request-key namespace.
        let continuation_dir = self.journal_continuation(request)?;
        if !continuation_dir {
            return self
                .journal
                .read(&request.request_id)?
                .ok_or_else(|| io::Error::other("missing launch receipt"));
        }
        let receipt = Self::new_dispatch(
            request,
            "local_consent_admitted_os_result_not_yet_confirmed",
        )?;
        self.journal.update(&receipt)?;
        let receipt = self.dispatch_claimed(request, native, receipt, deadline, valid)?;
        if receipt.state == LaunchState::ConsentRequired {
            self.journal
                .release_unstarted_continuation(&request.request_id)?;
        }
        Ok(receipt)
    }
    pub(super) fn journal_continuation(&self, request: &LaunchRequest) -> io::Result<bool> {
        self.journal.claim_continuation(&request.request_id)
    }
    pub(super) fn dispatch_claimed(
        &self,
        request: &LaunchRequest,
        native: &PlatformCatalog<P::Target>,
        mut receipt: LaunchReceipt,
        deadline: Option<i64>,
        valid: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) -> io::Result<LaunchReceipt> {
        let installed = native
            .apps
            .iter()
            .find(|a| a.app.app_id == request.app_id)
            .ok_or_else(|| io::Error::other("catalog entry vanished before dispatch"))?;
        if valid.as_ref().is_some_and(|check| !check()) {
            receipt.state = LaunchState::Denied;
            receipt.reason = "app_admission_revoked_or_cancelled".into();
        } else if !installed.app.can_launch {
            receipt.state = LaunchState::Unsupported;
            receipt.reason = installed
                .app
                .unavailable_reason
                .clone()
                .unwrap_or_else(|| "app_start_unavailable".into());
        } else {
            let known = self
                .windows
                .lock()
                .map_err(|_| io::Error::other("app window registry unavailable"))?
                .activate_known(request, deadline, &valid, || {
                    self.snapshot().is_ok_and(|(catalog, _)| {
                        catalog.session.unlocked && self.facts(request, &catalog).is_ok()
                    })
                });
            match known {
                Ok(Some(focused)) => {
                    receipt.state = LaunchState::StartedWindowPending;
                    receipt.reason = if focused {
                        "known_instance_activation_confirmed_p4_registration_pending"
                    } else {
                        "known_instance_available_activation_not_confirmed_p4_registration_pending"
                    }
                    .into();
                    self.journal.update(&receipt)?;
                    return Ok(receipt);
                }
                Err(error) => {
                    if error.to_string() == "app_activation_admission_ended" {
                        receipt.state = LaunchState::Denied;
                        receipt.reason = "app_activation_admission_ended".into();
                    } else {
                        receipt.reason =
                            format!("known_instance_activation_result_unknown: {error}");
                    }
                    self.journal.update(&receipt)?;
                    return Ok(receipt);
                }
                Ok(None) => {}
            }
            #[cfg(target_os = "android")]
            let _android_admission = super::super::platform::bridge::hold_admission(
                &receipt.dispatch_id,
                request.request_id.as_str(),
                deadline,
                valid,
            )?;
            match self.platform.dispatch(
                &installed.target,
                request,
                &native.session,
                deadline,
                &receipt.dispatch_id,
            ) {
                Ok(result) => {
                    receipt.state = result.state;
                    receipt.reason = result.reason;
                    receipt.pid = result.pid;
                }
                // An ambiguous OS/bridge error cannot prove that nothing started.
                Err(error) => {
                    receipt.reason = format!("os_dispatch_result_unknown: {error}");
                }
            }
            self.windows
                .lock()
                .map_err(|_| io::Error::other("app window registry unavailable"))?
                .take_started(request, &receipt.dispatch_id);
        }
        self.journal.update(&receipt)?;
        Ok(receipt)
    }
}
