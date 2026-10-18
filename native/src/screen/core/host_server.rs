//! One screen session on the host (SCR-M2, decisions C3, C5, C7). The share
//! host proved the viewer's relation and bound the connection; here the
//! session gets its rights (`capability_for` through `ScreenAuthority`),
//! registers with the registry (exclusive control), asks the host user on
//! „Fragen“ (`APPROVAL_PENDING` first), opens its source, shows the
//! indicator for screen content, answers `HELLO_OK` and runs: pictures
//! through the pipeline, the viewer's control frames, statistics once a
//! second. It ends when the viewer closes or stays silent, the host user or
//! a rights change stops it, or the picture side fails (`ERROR` with
//! `capture_failed` or `encoder_failed`). Every step is an access-log entry.
use std::io;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream};

use super::authorize::{
    ask, decide, log_event, Approval, AskContext, AuditSink, ScreenAuthority, SessionDecision,
};
use super::gop_stream::path_of;
use super::host_backend::{ConsentBroker, ScreenHostBackend, SessionIndicator};
use super::host_pipeline::Pipeline;
use super::negotiate::{negotiate, CodecPreference};
use super::registry::{NewSession, RegistryRefusal, ScreenRegistry, SessionHandle};
use super::session_state::{EndReason, SessionState};
use super::window_list::SourceId;
use super::wire_binary::StreamKind;
use super::wire_hello::{
    send_hello_reply, ApprovalKind, HelloReply, ScreenApprovalPending, ScreenAuthorization,
    ScreenClientHello, ScreenErrorCode, ScreenHelloError, ScreenHelloOk, SessionMode,
};
use crate::share::access_audit::ApprovalResult;
use crate::share::access_grant::{Capability, CapabilityState};
use crate::share::relation_proof::ProvenPrincipal;

#[path = "host_session.rs"]
mod session;
use session::Session;

/// Time the last frames of a session get before the connection closes.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// Everything a session uses of its host.
#[derive(Clone)]
pub(crate) struct HostContext {
    pub backend: Arc<dyn ScreenHostBackend>,
    pub registry: Arc<ScreenRegistry>,
    pub authority: Arc<dyn ScreenAuthority>,
    pub broker: Arc<dyn ConsentBroker>,
    pub indicator: Arc<dyn SessionIndicator>,
    pub audit: AuditSink,
    /// Test hook of the pipeline: picture streams to reset.
    pub inject_resets: Arc<AtomicU32>,
}

/// A viewer whose relation the share host proved.
pub(crate) struct ProvenViewer {
    pub connection: Connection,
    pub send: SendStream,
    pub recv: RecvStream,
    pub principal: ProvenPrincipal,
    pub device_name: String,
    pub hello: ScreenClientHello,
}

/// The start of a session: the proven viewer and its rights.
struct Start<'a> {
    context: &'a HostContext,
    connection: &'a Connection,
    send: &'a mut SendStream,
    principal: &'a ProvenPrincipal,
    mode: SessionMode,
}

impl Start<'_> {
    /// Refuses with `code`; `logged` when the cause has an entry already.
    async fn refuse(&mut self, code: ScreenErrorCode, logged: bool) -> io::Result<()> {
        if !logged {
            let reason = Some(code.key());
            log_event(
                self.context.audit,
                self.principal,
                self.mode,
                "session_refused",
                None,
                reason,
            );
        }
        let refused = HelloReply::Refused(ScreenHelloError::new(code));
        send_hello_reply(&mut *self.send, &refused).await?;
        let _ = self.send.finish();
        let _ = tokio::time::timeout(CLOSE_GRACE, self.connection.closed()).await;
        Ok(())
    }

    fn hello_ok(
        &self,
        authorization: ScreenAuthorization,
        approval: ApprovalKind,
    ) -> ScreenHelloOk {
        let backend = self.context.backend.as_ref();
        ScreenHelloOk {
            authorization,
            platform: backend.platform(),
            encoders: backend.encoders(),
            input: backend.input(),
            monitors: backend.monitors(),
            limits: backend.limits(),
            approval,
        }
    }

    /// Registers under the current rights; `None` when they no longer allow
    /// the session.
    fn register(&self, new: NewSession) -> Option<Result<SessionHandle, RegistryRefusal>> {
        let (mut new, mut registered) = (Some(new), None);
        let now = crate::share::core_now_secs();
        let needs = self.mode.capabilities();
        let admitted = self
            .context
            .authority
            .admit(self.principal, needs, now, &mut || {
                if let Some(new) = new.take() {
                    registered = Some(self.context.registry.register(new));
                }
            });
        admitted.then_some(registered).flatten()
    }

    /// Asks the host user; the kind of approval, or the refusal code.
    async fn confirm(
        &mut self,
        handle: &SessionHandle,
        capability: Capability,
        state: CapabilityState,
    ) -> Result<ApprovalKind, ScreenErrorCode> {
        let context = AskContext {
            authority: self.context.authority.as_ref(),
            broker: &self.context.broker,
            audit: self.context.audit,
            principal: self.principal,
            mode: self.mode,
            capability,
            policy_revision: state.policy_revision,
        };
        let pending = HelloReply::Pending(ScreenApprovalPending { capability });
        let send = &mut *self.send;
        let shown = async move {
            let _ = send_hello_reply(send, &pending).await;
        };
        let connection = self.connection;
        let gone = async move {
            let _ = connection.closed().await;
        };
        match ask(context, shown, gone, handle.stop_signal()).await {
            Approval::Granted(kind) => Ok(kind),
            Approval::Refused(code) => Err(code),
            Approval::Ended(EndReason::HostShutdown) => Err(ScreenErrorCode::HostShutdown),
            Approval::Ended(_) => Err(ScreenErrorCode::ApprovalCancelled),
        }
    }
}

/// Serves one session to its end.
pub(crate) async fn serve(context: HostContext, viewer: ProvenViewer) -> io::Result<()> {
    let ProvenViewer {
        connection,
        mut send,
        recv,
        principal,
        device_name,
        hello,
    } = viewer;
    let mut start = Start {
        context: &context,
        connection: &connection,
        send: &mut send,
        principal: &principal,
        mode: hello.mode,
    };
    let mode = hello.mode;
    let now = crate::share::core_now_secs();
    let rights = |capability| context.authority.capability(&principal, capability, now);
    let (Some(view), Some(control)) = (
        rights(Capability::ScreenView),
        rights(Capability::ScreenControl),
    ) else {
        return start.refuse(ScreenErrorCode::PermissionDenied, false).await;
    };
    let authorization = ScreenAuthorization {
        view: view.decision,
        control: control.decision,
    };
    let asked = match decide(mode, view.decision, control.decision) {
        SessionDecision::Refused(code) => return start.refuse(code, false).await,
        SessionDecision::InfoOnly => {
            let ok = start.hello_ok(authorization, ApprovalKind::NotRequired);
            send_hello_reply(&mut *start.send, &HelloReply::Accepted(ok)).await?;
            let _ = start.send.finish();
            let _ = tokio::time::timeout(CLOSE_GRACE, connection.closed()).await;
            return Ok(());
        }
        SessionDecision::Allowed => None,
        SessionDecision::Ask(capability) => Some(capability),
    };
    let backend = context.backend.as_ref();
    let Some(source_id) = hello.source.clone().or_else(|| primary_monitor(backend)) else {
        return start.refuse(ScreenErrorCode::NoDisplay, false).await;
    };
    let screen_content = source_id.is_screen_content();
    if screen_content && !backend.graphical_session() {
        return start.refuse(ScreenErrorCode::NoDisplay, false).await;
    }
    if screen_content && !context.indicator.available() {
        return start
            .refuse(ScreenErrorCode::IndicatorUnavailable, false)
            .await;
    }
    if mode == SessionMode::Control && context.registry.controller().is_some() {
        return start.refuse(ScreenErrorCode::ControlBusy, false).await;
    }
    let new = NewSession {
        principal: principal.clone(),
        device_name,
        mode,
        source: source_id.to_string(),
        path: path_of(&connection).0.key().into(),
        started_at: now,
        state: match asked {
            Some(_) => SessionState::WaitingForApproval,
            None => SessionState::Starting,
        },
        screen_content,
    };
    let handle = match start.register(new) {
        Some(Ok(handle)) => handle,
        Some(Err(RegistryRefusal::SessionLimit)) => {
            return start.refuse(ScreenErrorCode::SessionLimit, false).await;
        }
        None => return start.refuse(ScreenErrorCode::PermissionDenied, false).await,
    };
    let approval = match asked {
        None => ApprovalKind::NotRequired,
        Some(capability) => {
            let state = if capability == Capability::ScreenControl {
                control
            } else {
                view
            };
            match start.confirm(&handle, capability, state).await {
                Ok(kind) => kind,
                Err(code) => return start.refuse(code, true).await,
            }
        }
    };
    let source = match backend.open_source(&source_id, hello.display_size) {
        Ok(source) => source,
        Err(refusal) => {
            let reason = Some(refusal.reason.as_str());
            log_event(
                context.audit,
                &principal,
                mode,
                "session_refused",
                None,
                reason,
            );
            return start.refuse(ScreenErrorCode::NoDisplay, true).await;
        }
    };
    if mode == SessionMode::Control && handle.take_control().is_err() {
        return start.refuse(ScreenErrorCode::ControlBusy, false).await;
    }
    if screen_content {
        if let Err(error) = context.indicator.show() {
            let reason = Some(error.as_str());
            log_event(
                context.audit,
                &principal,
                mode,
                "session_refused",
                None,
                reason,
            );
            return start
                .refuse(ScreenErrorCode::IndicatorUnavailable, true)
                .await;
        }
    }
    let choices = negotiate(&backend.encoders(), &hello.decoders, CodecPreference::Auto);
    let Some(choice) = choices.first().copied() else {
        return start.refuse(ScreenErrorCode::EncoderFailed, false).await;
    };
    let ok = start.hello_ok(authorization, approval);
    send_hello_reply(&mut send, &HelloReply::Accepted(ok)).await?;
    let result = Some(approval_result(approval));
    log_event(
        context.audit,
        &principal,
        mode,
        "session_started",
        result,
        None,
    );
    if mode == SessionMode::Control {
        log_event(context.audit, &principal, mode, "control_taken", None, None);
    }
    let max_fps = u32::from(hello.max_fps).clamp(1, super::pacing::MAX_FPS);
    let mut pipeline = Pipeline::new(
        context.backend.clone(),
        source,
        choice,
        &connection,
        max_fps,
        hello.display_size,
        context.inject_resets.clone(),
    );
    if mode == SessionMode::Preview {
        pipeline.set_mode(StreamKind::Preview, None, None);
    }
    let session = Session::new(
        &context,
        &connection,
        send,
        handle,
        &principal,
        mode,
        pipeline,
    );
    session.run(recv, max_fps).await;
    Ok(())
}

fn primary_monitor(backend: &dyn ScreenHostBackend) -> Option<SourceId> {
    let monitors = backend.monitors();
    let primary = monitors
        .iter()
        .find(|monitor| monitor.primary)
        .or_else(|| monitors.first())?;
    Some(SourceId::Monitor(primary.id.clone()))
}

fn approval_result(approval: ApprovalKind) -> ApprovalResult {
    match approval {
        ApprovalKind::NotRequired => ApprovalResult::NotRequired,
        ApprovalKind::Once => ApprovalResult::Once,
        ApprovalKind::Always => ApprovalResult::Always,
    }
}
