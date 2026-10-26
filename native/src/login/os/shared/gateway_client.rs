//! Remote viewer client on its existing user endpoint; every effect is peer-signed.
use std::{io, sync::Arc, time::{Duration, Instant}};
use iroh::endpoint::{RecvStream, SendStream};
use crate::{login::core::{gateway_types::{ClientHello, GatewayAccess, GatewayReply, GatewayTarget, ServerHello}, gateway_wire as wire,
    model::{Challenge, Operation, SignedRequest, VERSION}}, share::login_transport::{self as transport, LoginClientContext, RouteCancellation, RouteStatus}};
use super::{denied, now};
use crate::{login::core::image_receive::ImageReceiver, screen::core::{capture_admission::SourceKey, capture_grant::CaptureCreditLifetime, capture_image::CapturedImage}};
const SETUP: Duration = Duration::from_secs(30);
const OPERATION: Duration = Duration::from_secs(30);

pub(crate) struct RemoteLoginClient {
    connection: transport::Close,
    send: SendStream,
    recv: RecvStream,
    context: LoginClientContext,
    target: GatewayTarget,
    user: String,
    challenge: Option<Challenge>,
    access: GatewayAccess,
    cancel: Arc<RouteCancellation>,
    closed: bool,
    images: ImageReceiver,
}
impl RemoteLoginClient {
    /// Blocking daemon/UI worker entrypoint; async owners use connect_async.
    pub(crate) fn connect(context: LoginClientContext, target: GatewayTarget, target_user: String,
        cancel: Arc<RouteCancellation>) -> io::Result<Self> {
        let runtime = context.runtime.clone(); runtime.block_on(Self::connect_async(context, target, target_user, cancel))
    }
    pub(crate) async fn connect_async(context: LoginClientContext, target: GatewayTarget, target_user: String,
        cancel: Arc<RouteCancellation>) -> io::Result<Self> {
        Self::connect_async_owned(context, target, target_user, cancel, None).await
    }
    pub(crate) async fn connect_async_owned(context: LoginClientContext, target: GatewayTarget, target_user: String,
        cancel: Arc<RouteCancellation>, lifetime: Option<Arc<dyn CaptureCreditLifetime>>) -> io::Result<Self> {
        let admission = crate::screen::os::image_memory::admit_images(ImageReceiver::limits()?)?;
        let receive_cancel = cancel.clone();
        let images = ImageReceiver {admission, source: SourceKey::new(&super::nonce()?, "login:network")?,
            cancelled: Arc::new(move || receive_cancel.check().is_err()), lifetime};
        target.validate()?;
        if target_user.is_empty() || target_user.len() > 256 { return Err(denied("login target user is empty or exceeds limit")); }
        let deadline = Instant::now() + SETUP;
        let close = transport::connect(&context, &target, &cancel, deadline).await?;
        let connection = &close.0;
        let (mut send, mut recv) = cancelled_route(connection, &cancel, deadline,
            async { connection.accept_bi().await.map_err(io::Error::other) }).await?;
        let server: ServerHello = cancelled_route(connection, &cancel, deadline, wire::receive(&mut recv)).await?;
        if server.format != VERSION || server.route_policy != VERSION || server.machine_id != target.machine_id
            || server.public_key != target.public_key || connection.remote_id().to_string() != target.public_key {
            return Err(denied("login machine identity or route-policy version differs from its administrative pin"));
        }
        if server.admission == "permission_missing" { return Err(denied("permission_missing: administrative login and screen-view rights are missing or expired for this peer")); }
        if server.admission != "ready" { return Err(denied("login gateway admission state is unsupported")); }
        transport::authenticated(connection);
        let hello = ClientHello { format: VERSION, peer_id: context.peer_id.clone(), target_user: target_user.clone(), route_policy: VERSION };
        cancelled_route(connection, &cancel, deadline, wire::send(&mut send, &hello)).await?;
        let (initial, pixels) = cancelled_route(connection, &cancel, deadline, wire::receive_reply(&mut recv, &images)).await?;
        if initial.request_sequence != 0 || pixels.is_some() { return Err(denied("unexpected login startup response")); }
        let challenge = initial.reply.challenge.ok_or_else(|| denied(format!("{}: {}", initial.reply.state, initial.reply.reason.unwrap_or_else(|| "login broker is unavailable".into()))))?;
        target.check_challenge(&challenge, &target_user)?;
        if now()? > challenge.expires_at { return Err(denied("login startup challenge expired")); }
        let access = initial.access.ok_or_else(|| denied("login gateway returned no authenticated administrative access report"))?;
        if access.peer_id != context.peer_id || access.public_key != context.key.public().to_string()
            || access.target_user != target_user || access.policy_revision != challenge.policy_revision
            || access.not_before > now()? || access.expires_at <= now()? || !access.rights.login || !access.rights.screen_view {
            return Err(denied("login access report differs from the authenticated peer, user, policy or lifetime"));
        }
        Ok(Self { connection: close, send, recv, context, target, user: target_user, challenge: Some(challenge), access, cancel, closed: false, images })
    }
    pub(crate) fn challenge(&self) -> io::Result<&Challenge> {
        self.challenge.as_ref().filter(|_| !self.closed).ok_or_else(|| denied("remote login session has no live challenge"))
    }
    pub(crate) fn access(&self) -> &GatewayAccess { &self.access }
    pub(crate) async fn wait_closed(&self) { let _ = self.connection.0.closed().await; }
    pub(crate) fn signed_request(&self, operation: Operation) -> io::Result<SignedRequest> {
        operation.validate().map_err(denied)?;
        let mut request = SignedRequest { challenge: self.challenge()?.clone(), peer_id: self.context.peer_id.clone(),
            target_user: self.user.clone(), operation, signature: String::new() };
        request.sign(&self.context.key)?; Ok(request)
    }
    pub(crate) fn signed_operation(&mut self, request: SignedRequest) -> io::Result<(GatewayReply, Option<CapturedImage>)> {
        let runtime = self.context.runtime.clone(); runtime.block_on(self.signed_operation_async(request))
    }
    pub(crate) async fn signed_operation_async(&mut self, request: SignedRequest) -> io::Result<(GatewayReply, Option<CapturedImage>)> {
        if self.closed { return Err(denied("remote login session is closed")); }
        let current = self.challenge()?.clone();
        let hello = ClientHello { format: VERSION, peer_id: self.context.peer_id.clone(), target_user: self.user.clone(), route_policy: VERSION };
        hello.check_operation(&request, &self.context.key.public().to_string(), &current)?;
        if now()? > current.expires_at { self.close(); return Err(denied("login challenge expired; reconnect before retrying an operation")); }
        let sequence = current.sequence;
        let is_stop = matches!(request.operation, Operation::Stop);
        let deadline = Instant::now() + OPERATION;
        let route_epoch = transport::status(&self.connection.0).map(|status| status.pause_epoch);
        let result = (async {
            cancelled_route(&self.connection.0, &self.cancel, deadline, wire::send(&mut self.send, &request)).await?;
            // Construct once. A route pause retains this exact signed request;
            // no retry can replay an already consumed sequence or input bundle.
            let (packet, pixels) = cancelled_route(&self.connection.0, &self.cancel, deadline, wire::receive_reply(&mut self.recv, &self.images)).await?;
            if packet.request_sequence != sequence { return Err(denied("login response sequence does not match the signed request")); }
            if let Some(next) = &packet.reply.challenge {
                self.target.check_challenge(next, &self.user)?;
                if next.boot_generation != current.boot_generation || next.binding != current.binding
                    || next.policy_revision != current.policy_revision || next.sequence != sequence.checked_add(1).ok_or_else(|| denied("login sequence exhausted"))? {
                    return Err(denied("login response changed its signed boot, session, desktop or policy generation"));
                }
            }
            // Pixels captured before an interrupted path are discarded. Effect
            // completion is never inferred from this display-generation check.
            if pixels.is_some() && transport::status(&self.connection.0).map(|status| status.pause_epoch) != route_epoch {
                return Err(denied("login route generation changed while an image frame was in flight; reconnect"));
            }
            Ok((packet, pixels))
        }).await;
        match result {
            Ok((packet, pixels)) => {
                self.challenge = packet.reply.challenge.clone();
                if is_stop || self.challenge.is_none() { self.close(); }
                Ok((packet, pixels))
            }
            Err(error) => { self.close(); Err(error) }
        }
    }
    pub(crate) fn operation(&mut self, operation: Operation) -> io::Result<(GatewayReply, Option<CapturedImage>)> {
        let request = self.signed_request(operation)?; self.signed_operation(request)
    }
    pub(crate) fn route_status(&self) -> Option<RouteStatus> { transport::status(&self.connection.0) }
    /// Immediate owner cancellation also works while the sender is path-blocked.
    /// Local gateway EOF then ends the broker/helper and releases held inputs.
    pub(crate) fn close(&mut self) {
        if !self.closed { self.closed = true; self.challenge = None; self.cancel.cancel(); transport::close(&self.connection.0); }
    }
}
impl Drop for RemoteLoginClient { fn drop(&mut self) { self.close(); } }
async fn cancelled_route<T>(connection: &iroh::endpoint::Connection, cancel: &Arc<RouteCancellation>, deadline: Instant,
    future: impl std::future::Future<Output = io::Result<T>>) -> io::Result<T> {
    cancel.check().map_err(|error| if error.kind() == io::ErrorKind::Interrupted { io::Error::new(io::ErrorKind::Interrupted, "remote login operation cancelled") } else { error })?;
    tokio::select! {
        biased;
        error = transport::cancelled(cancel) => Err(error),
        result = transport::routed(connection, OPERATION, deadline, future) => result,
    }
}
