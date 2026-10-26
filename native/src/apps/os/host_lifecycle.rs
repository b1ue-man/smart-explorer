//! Read-only receipts, local completion and authenticated P4 window binding.
use super::*;

impl<P: HostPlatform> AppHost<P> {
    /// Read-only recovery after timeout/lost response. Main checks permission
    /// before returning this local receipt to a remote peer.
    pub(crate) fn receipt(&self, request: &LaunchRequest) -> io::Result<Option<LaunchReceipt>> {
        if request.target_device_id != self.device {
            return Err(invalid("receipt target device identity changed"));
        }
        let receipt = self.journal.read(&request.request_id)?;
        if receipt.as_ref().is_some_and(|p| p.request != *request) {
            return Err(invalid("request ID contents differ"));
        }
        Ok(receipt)
    }
    /// Observation-only continuation/recovery. The caller supplies the exact
    /// authenticated P4 owner; registration gives no ScreenView/Control grant.
    pub(crate) fn correlate_window(
        &self,
        request: &LaunchRequest,
        register: impl FnOnce(
            crate::remote_window::WindowPin,
        )
            -> io::Result<crate::screen::core::family_protocol::FamilyListEntry>,
    ) -> io::Result<Option<LaunchReceipt>> {
        let Some(receipt) = self.receipt(request)? else {
            return Ok(None);
        };
        if !matches!(
            receipt.state,
            LaunchState::OutcomeUnknown
                | LaunchState::StartedWindowPending
                | LaunchState::Started
                | LaunchState::Activated
        ) {
            return Ok(Some(receipt));
        }
        let session = self.platform.session();
        if !session.is_ok_and(|session| {
            session.user_id == request.user_id
                && session.session_id == request.session_id
                && session.interactive
        }) {
            return self
                .window_pending(request, "original_app_logon_or_user_ended")
                .map(Some);
        }
        let result = self
            .windows
            .lock()
            .map_err(|_| io::Error::other("app window registry unavailable"))?
            .confirm(request, register);
        let confirmed=match result {
            Ok(Some(confirmed))=>confirmed,
            Ok(None)=>return self.window_pending(request,"original_process_witness_unavailable; no_window_or_projection_selection_invented").map(Some),
            Err(error)=>return self.window_pending(request,&format!("window_not_confirmed: {error}")).map(Some),
        };
        let entry = &confirmed.entry;
        if entry.launch_request.as_deref() != Some(request.request_id.as_str())
            || crate::screen::core::family_protocol::selector(&entry.source)
                != Some(entry.identity.session.as_str())
            || entry.identity.generation != 1
        {
            return self
                .window_pending(request, "p4_registration_request_or_identity_mismatch")
                .map(Some);
        }
        let crate::screen::core::window_list::SourceId::Window(window_id) = &entry.source else {
            return self
                .window_pending(request, "p4_registration_is_not_a_window")
                .map(Some);
        };
        match self.note_window(
            request,
            WindowBinding {
                app_id: request.app_id.clone(),
                user_id: request.user_id.clone(),
                session_id: request.session_id.clone(),
                window_id: window_id.clone(),
                generation: entry.identity.generation,
            },
            confirmed.focused,
        ) {
            Ok(receipt) => Ok(Some(receipt)),
            Err(error) => self
                .window_pending(request, &format!("p4_window_binding_refused: {error}"))
                .map(Some),
        }
    }
    pub(crate) fn has_window_process(&self, request: &LaunchRequest) -> bool {
        self.windows
            .lock()
            .is_ok_and(|windows| windows.has_process(request))
    }
    pub(super) fn window_pending(&self, request: &LaunchRequest, reason: &str) -> io::Result<LaunchReceipt> {
        let _serial = self
            .serial
            .lock()
            .map_err(|_| io::Error::other("launch serialization poisoned"))?;
        let mut receipt = self
            .receipt(request)?
            .ok_or_else(|| invalid("unknown app window request"))?;
        if matches!(
            receipt.state,
            LaunchState::OutcomeUnknown
                | LaunchState::StartedWindowPending
                | LaunchState::Started
                | LaunchState::Activated
        ) {
            // OutcomeUnknown still means the OS dispatch may have occurred. It
            // must never be promoted to Started merely because probing failed.
            if receipt.state != LaunchState::OutcomeUnknown {
                receipt.state = LaunchState::StartedWindowPending;
            }
            receipt.window = None;
            receipt.reason = reason.into();
            self.journal.update(&receipt)?;
        }
        Ok(receipt)
    }
    /// Trusted local Android bridge completion, including a reply arriving after
    /// the synchronous dispatch timed out. It cannot trigger an OS operation.
    pub(crate) fn note_dispatch_result(
        &self,
        request: &LaunchRequest,
        dispatch_id: &str,
        result: super::super::shared::platform::DispatchReceipt,
    ) -> io::Result<LaunchReceipt> {
        let _serial = self
            .serial
            .lock()
            .map_err(|_| io::Error::other("launch serialization poisoned"))?;
        let mut receipt = self
            .receipt(request)?
            .ok_or_else(|| invalid("unknown local platform completion"))?;
        if receipt.dispatch_id != dispatch_id {
            return Ok(receipt);
        }
        if receipt.state == LaunchState::ConsentRequired && result.state == LaunchState::Denied {
            receipt.state = result.state;
            receipt.reason = result.reason;
            self.journal.update(&receipt)?;
            return Ok(receipt);
        }
        if receipt.state != LaunchState::OutcomeUnknown {
            return Ok(receipt);
        }
        if result.state == LaunchState::OutcomeUnknown {
            return Ok(receipt);
        }
        if !matches!(
            result.state,
            LaunchState::StartedWindowPending
                | LaunchState::ConsentRequired
                | LaunchState::Denied
                | LaunchState::Unsupported
        ) {
            return Err(invalid(
                "platform completion cannot assert a P4 window activation",
            ));
        }
        receipt.state = result.state;
        receipt.reason = result.reason;
        receipt.pid = result.pid;
        self.journal.update(&receipt)?;
        if receipt.state == LaunchState::ConsentRequired {
            self.journal
                .release_unstarted_continuation(&request.request_id)?;
        }
        Ok(receipt)
    }
    /// Called by the local P4 adapter after verified window/app/session matching.
    /// A PID/name match alone is insufficient. Timeout never calls `launch` again.
    pub(crate) fn note_window(
        &self,
        request: &LaunchRequest,
        window: WindowBinding,
        activated: bool,
    ) -> io::Result<LaunchReceipt> {
        let _serial = self
            .serial
            .lock()
            .map_err(|_| io::Error::other("launch serialization poisoned"))?;
        if window.app_id != request.app_id
            || window.user_id != request.user_id
            || window.session_id != request.session_id
            || window.window_id.is_empty()
            || window.generation == 0
        {
            return Err(invalid("P4 window does not belong to this launch"));
        }
        let (catalog, _) = self.snapshot()?;
        self.facts(request, &catalog).map_err(invalid)?;
        let mut receipt = self
            .receipt(request)?
            .ok_or_else(|| invalid("unknown launch request"))?;
        if !matches!(
            receipt.state,
            LaunchState::StartedWindowPending
                | LaunchState::Started
                | LaunchState::Activated
                | LaunchState::OutcomeUnknown
        ) {
            return Err(invalid("launch has no admitted window transition"));
        }
        receipt.state = if activated {
            LaunchState::Activated
        } else {
            LaunchState::Started
        };
        let known = receipt.reason.starts_with("known_instance_")
            || receipt.reason.starts_with("existing_instance_");
        receipt.reason = if activated {
            "existing_or_started_window_activation_confirmed"
        } else if known {
            "existing_instance_window_available_activation_not_confirmed"
        } else {
            "started_window_available"
        }
        .into();
        receipt.window = Some(window);
        self.journal.update(&receipt)?;
        Ok(receipt)
    }
}
