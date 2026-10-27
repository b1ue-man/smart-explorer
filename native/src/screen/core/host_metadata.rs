//! Shared async boundary and publication guard for screen metadata.
use super::app_icon::{IconRequest, IconResponse};
use super::frame_memory::{CapacityCeiling, MemoryFacts};
use super::host_metadata_jobs::{self as jobs, Job};
use super::host_server::HostContext;
use super::input_model::InputCapabilities;
use super::negotiate::CodecOffer;
use super::session_state::EndReason;
use super::window_list::{MonitorInfo, SourceId};
use super::wire_control::HostFrame;
use super::wire_hello::{HelloReply, HostPlatform, SessionMode};
use crate::share::{access_grant::Capability, relation_proof::ProvenPrincipal};
use iroh::endpoint::{Connection, SendStream};
use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::{io::AsyncWrite, sync::watch};

pub(crate) struct HostMetadata {
    pub platform: HostPlatform,
    pub encoders: Vec<CodecOffer>,
    pub input: InputCapabilities,
    pub monitors: Vec<MonitorInfo>,
    pub limits: Vec<String>,
}

impl HostMetadata {
    pub(crate) fn query(
        context: &HostContext,
        principal: &ProvenPrincipal,
        mode: SessionMode,
    ) -> Job<Self> {
        jobs::spawn_start(context, principal, mode.capabilities(), |context, _| {
            let backend = context.backend.as_ref();
            Ok(Self {
                platform: backend.platform(),
                encoders: backend.encoders(),
                input: backend.input(),
                monitors: backend.monitors(),
                limits: backend.limits(),
            })
        })
    }
}

pub(crate) struct CaptureResources {
    pub facts: Option<MemoryFacts>,
    pub ceiling: CapacityCeiling,
    pub session_memory_ceiling: u64,
}

/// Only an actual capture startup observes and admits capture memory.
pub(crate) fn capture_resources(
    context: &HostContext,
    principal: &ProvenPrincipal,
    mode: SessionMode,
) -> Job<CaptureResources> {
    jobs::spawn_start(context, principal, mode.capabilities(), |context, _| {
        let backend = context.backend.as_ref();
        let facts = backend.memory_facts();
        Ok(CaptureResources {
            facts,
            ceiling: backend.capacity_ceiling(facts),
            session_memory_ceiling: backend.session_memory_ceiling(),
        })
    })
}

/// Source discovery precedes consent; codec/memory discovery stays after consent.
pub(crate) struct SourceOptions {
    pub source: Option<SourceId>,
    pub graphical: bool,
    pub indicator_available: bool,
}

pub(crate) fn source_options(
    context: &HostContext,
    principal: &ProvenPrincipal,
    mode: SessionMode,
    source: Option<SourceId>,
) -> Job<SourceOptions> {
    jobs::spawn_start(
        context,
        principal,
        mode.capabilities(),
        move |context, _| {
            let backend = context.backend.as_ref();
            let source = source.or_else(|| {
                let monitors = backend.monitors();
                monitors
                    .iter()
                    .find(|monitor| monitor.primary)
                    .or_else(|| monitors.first())
                    .map(|monitor| SourceId::Monitor(monitor.id.clone()))
            });
            Ok(SourceOptions {
                source,
                graphical: backend.graphical_session(),
                indicator_available: context.indicator.available(),
            })
        },
    )
}

pub(crate) enum Request {
    Windows,
    Icon(IconRequest),
    Families,
}
impl Request {
    pub(crate) fn start(
        self,
        context: &HostContext,
        principal: &ProvenPrincipal,
    ) -> Job<HostFrame> {
        jobs::spawn(
            context,
            principal,
            &[Capability::ScreenView],
            move |context, principal| match self {
                Self::Windows => super::icon_host::list(context, principal).map(HostFrame::Windows),
                Self::Icon(request) => {
                    super::icon_host::icon(context, principal, request).map(HostFrame::IconReply)
                }
                Self::Families => {
                    super::family_listing::list(context, principal).map(HostFrame::Families)
                }
            },
        )
    }

    pub(crate) fn fallback(&self) -> Fallback {
        match self {
            Self::Windows => Fallback::Windows,
            Self::Icon(request) => Fallback::Icon(request.clone()),
            Self::Families => Fallback::Families,
        }
    }
}

/// The kind of a finished request, kept for answering its failure.
#[derive(Clone)]
pub(crate) enum Fallback {
    Windows,
    Icon(IconRequest),
    Families,
}
impl Fallback {
    /// An icon whose lookup ran out of time or whose platform step failed is
    /// omitted, as every platform icon failure always was; a lost right, a
    /// stop and every other failure stay errors.
    pub(crate) fn settle(self, result: io::Result<HostFrame>) -> io::Result<HostFrame> {
        match (self, result) {
            (Self::Icon(request), Err(error))
                if matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::Other) =>
            {
                Ok(HostFrame::IconReply(IconResponse {
                    request,
                    content: None,
                }))
            }
            (_, result) => result,
        }
    }
}

/// Metadata stays FIFO; capture, pings and control release keep their own session loop.
pub(crate) struct Requests {
    pending: VecDeque<Request>,
    running: Option<(Fallback, Job<HostFrame>)>,
    capacity: usize,
}
impl Requests {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            pending: VecDeque::new(),
            running: None,
            capacity,
        }
    }
    pub(crate) fn can_accept(&self) -> bool {
        self.pending.len() < self.capacity
    }
    pub(crate) fn push(
        &mut self,
        context: &HostContext,
        principal: &ProvenPrincipal,
        request: Request,
    ) -> io::Result<()> {
        if !self.can_accept() {
            return Err(io::Error::from(io::ErrorKind::WouldBlock));
        }
        self.pending.push_back(request);
        self.advance(context, principal);
        Ok(())
    }
    pub(crate) fn advance(&mut self, context: &HostContext, principal: &ProvenPrincipal) {
        if self.running.is_none() {
            if let Some(request) = self.pending.pop_front() {
                let fallback = request.fallback();
                self.running = Some((fallback, request.start(context, principal)));
            }
        }
    }
    /// Cancel-safe: an unfinished job stays running for the next call.
    pub(crate) async fn next(&mut self) -> (Fallback, io::Result<HostFrame>) {
        let Some((fallback, job)) = self.running.as_mut() else {
            return std::future::pending().await;
        };
        let result = job.result().await;
        let fallback = fallback.clone();
        self.running = None;
        (fallback, result)
    }
    pub(crate) fn cancel(&mut self) {
        self.pending.clear();
        self.running = None;
    }
}

fn ended(stop: Option<&watch::Receiver<Option<EndReason>>>) -> bool {
    stop.is_some_and(|stop| stop.borrow().is_some() || stop.has_changed().is_err())
}

async fn stopped(stop: Option<&mut watch::Receiver<Option<EndReason>>>) {
    match stop {
        Some(stop) => {
            let _ = stop.changed().await;
        }
        None => std::future::pending().await,
    }
}

/// `Interrupted` when `stop` ends the session, `ConnectionAborted` when the viewer leaves.
pub(crate) async fn until<T>(
    connection: &Connection,
    mut stop: Option<watch::Receiver<Option<EndReason>>>,
    work: impl Future<Output = io::Result<T>>,
) -> io::Result<T> {
    if ended(stop.as_ref()) {
        return Err(io::Error::from(io::ErrorKind::Interrupted));
    }
    tokio::select! { biased;
        _ = stopped(stop.as_mut()) => Err(io::Error::from(io::ErrorKind::Interrupted)),
        _ = connection.closed() => Err(io::Error::from(io::ErrorKind::ConnectionAborted)),
        result = work => result,
    }
}

/// Authorization covers each actual transport write, never an async wait or OS query.
pub(crate) struct AuthorizedWriter<'a, W> {
    pub writer: &'a mut W,
    pub context: &'a HostContext,
    pub principal: &'a ProvenPrincipal,
    pub mode: SessionMode,
    pub stop: Option<&'a watch::Receiver<Option<EndReason>>>,
}
impl<W> AuthorizedWriter<'_, W> {
    fn admitted<T>(
        &mut self,
        mut poll: impl FnMut(&mut W) -> Poll<io::Result<T>>,
    ) -> Poll<io::Result<T>> {
        let mut result = Poll::Ready(Err(io::Error::from(io::ErrorKind::Interrupted)));
        let stop = self.stop;
        let writer = &mut *self.writer;
        let allowed = self.context.authority.admit(
            self.principal,
            self.mode.capabilities(),
            crate::share::core_now_secs(),
            &mut || {
                if !ended(stop) {
                    result = poll(writer);
                }
            },
        );
        if !allowed {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "screen metadata publication revoked",
            )));
        }
        result
    }
}
impl<W: AsyncWrite + Unpin> AsyncWrite for AuthorizedWriter<'_, W> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.get_mut()
            .admitted(|writer| Pin::new(writer).poll_write(cx, bytes))
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut()
            .admitted(|writer| Pin::new(writer).poll_flush(cx))
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut()
            .admitted(|writer| Pin::new(writer).poll_shutdown(cx))
    }
}

/// One metadata frame; every write needs `mode`'s rights and a running session.
pub(crate) async fn send(
    context: &HostContext,
    principal: &ProvenPrincipal,
    mode: SessionMode,
    connection: &Connection,
    send: &mut SendStream,
    response: &HostFrame,
    stop: watch::Receiver<Option<EndReason>>,
) -> io::Result<()> {
    let mut writer = AuthorizedWriter {
        writer: send,
        context,
        principal,
        mode,
        stop: Some(&stop),
    };
    let sent =
        super::control_sender::send(connection.remote_id().to_string(), &mut writer, response);
    until(connection, Some(stop.clone()), sent).await
}

/// `HELLO_OK`; every write needs `mode`'s rights. A stop is left to the
/// session loop, which reports it to the viewer as before.
pub(crate) async fn hello(
    context: &HostContext,
    principal: &ProvenPrincipal,
    mode: SessionMode,
    connection: &Connection,
    send: &mut SendStream,
    response: &HelloReply,
) -> io::Result<()> {
    let mut writer = AuthorizedWriter {
        writer: send,
        context,
        principal,
        mode,
        stop: None,
    };
    let sent = super::wire_hello::send_hello_reply(&mut writer, response);
    until(connection, None, sent).await
}

/// Remote-suite case of the publication guard; fixtures as in the session cases.
#[cfg(test)]
mod publication_tests {
    use super::*;
    use crate::screen::core::screen_metadata_runtime_fixtures::{self as f, bounded};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::task::Waker;

    #[derive(Default)]
    struct WriteState {
        ready: AtomicBool,
        entered: tokio::sync::Notify,
        waker: Mutex<Option<Waker>>,
        bytes: Mutex<Vec<u8>>,
    }
    struct Writer(Arc<WriteState>);
    impl AsyncWrite for Writer {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            if !self.0.ready.load(Ordering::SeqCst) {
                *self.0.waker.lock().unwrap() = Some(cx.waker().clone());
                self.0.entered.notify_one();
                return Poll::Pending;
            }
            self.0.bytes.lock().unwrap().extend_from_slice(bytes);
            Poll::Ready(Ok(bytes.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[test]
    fn screen_metadata_runtime_task_publication_rechecks_rights_and_stop_after_backpressure() {
        f::run(false, async {
            let cases = [
                (SessionMode::View, Some((false, true))),
                (SessionMode::Control, Some((true, false))),
                (SessionMode::View, None),
            ];
            for (mode, revoke) in cases {
                let rig = f::rig();
                let state = Arc::new(WriteState::default());
                let mut writer = Writer(state.clone());
                let (stop, signal) = tokio::sync::watch::channel(None);
                let (context, principal) = (&rig.context, &rig.principal);
                let writer = &mut writer;
                let stop_signal = Some(&signal);
                let mut guarded = AuthorizedWriter {
                    writer,
                    context,
                    principal,
                    mode,
                    stop: stop_signal,
                };
                let request = rig.backend.icon_request(true);
                let content = Some(rig.backend.reply.clone());
                let response = HostFrame::IconReply(IconResponse { request, content });
                let peer = "metadata-fixture-peer".to_string();
                let sent = crate::screen::core::control_sender::send(peer, &mut guarded, &response);
                let change = async {
                    bounded(state.entered.notified()).await;
                    match revoke {
                        Some(rights) => f::set_rights(&rig, rights).await,
                        None => {
                            stop.send_replace(Some(EndReason::HostStopped));
                        }
                    }
                    state.ready.store(true, Ordering::SeqCst);
                    if let Some(waker) = state.waker.lock().unwrap().take() {
                        waker.wake();
                    }
                };
                let (result, ()) = bounded(async { tokio::join!(sent, change) }).await;
                let expected = match revoke {
                    Some(_) => io::ErrorKind::PermissionDenied,
                    None => io::ErrorKind::Interrupted,
                };
                assert_eq!(result.unwrap_err().kind(), expected);
                assert!(state.bytes.lock().unwrap().is_empty());
            }
        });
    }
}
