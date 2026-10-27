//! The portal screen cast of a Wayland host (SCR-M5, U19-4): the session
//! with `org.freedesktop.portal.ScreenCast` through `ashpd` 0.13, the user's
//! choice in the system dialog, the restore token, and the PipeWire stream
//! (`pipewire_stream`). It runs in the host process on a thread of its own
//! (`portal-session`, a current-thread runtime); opening the source never
//! blocks: until the dialog is answered and the stream delivers, `capture`
//! reports `WaitingForPortal`. Neither the dialog nor the wait has a time
//! limit of its own: the user's answer, the end of the session (dropping the
//! handle) or a failure of the portal ends it. The state lives in the
//! session, not in the capture: a session that may control
//! (`PortalOptions::remote_desktop`, SCR-M6) runs as a `RemoteDesktop`
//! session with the screen cast in it (`portal_input`).
//!
//! Assumption: the `ashpd` 0.13 names and option types are those of its
//! documentation (docs.rs, 2026-10-19); they could not be built here.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::pipewire_abi;
use super::pipewire_stream::memory::CaptureMemory;
use super::pipewire_stream::{LatestFrame, PwStream, Taken};
use super::portal_tokens::{self, TokenKind};
use crate::screen::core::capture_admission::{CaptureGrant, SessionAdmission, SourceKey};
use crate::screen::core::host_backend::{ScreenSource, SourceFrame};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::SessionState;

#[path = "portal_connection.rs"]
pub(super) mod connection;
#[path = "portal_source_binding.rs"]
pub(super) mod source_binding;

/// Size of a portal source until the first picture names the real one.
pub(crate) const PORTAL_PROVISIONAL: FrameSize = FrameSize::new(1280, 720);
/// The failure text of a system without PipeWire.
pub(crate) const PIPEWIRE_MISSING: &str = "pipewire_missing";

/// What the portal session is doing.
pub(crate) enum PortalState {
    /// The dialog may be open; the stream is not there yet.
    Waiting,
    WaitingInactive,
    Streaming(Arc<LatestFrame>),
    /// The user declined (response code 1 or 2 of the portal).
    Denied(String),
    /// The portal service, PipeWire or the stream failed.
    Failed(String),
}

/// How to start the portal session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PortalOptions {
    /// Input through the `RemoteDesktop` portal in the same session (U22).
    pub remote_desktop: bool,
    /// The PipeWire shared object (tests name a wrong one).
    pub pipewire_library: &'static str,
}

impl Default for PortalOptions {
    fn default() -> Self {
        Self {
            remote_desktop: false,
            pipewire_library: pipewire_abi::LIBRARY_NAME,
        }
    }
}

/// The running portal session; dropping it ends the session.
pub(crate) struct PortalHandle {
    state: Arc<Mutex<PortalState>>,
    /// Dropped (or sent to) to end the session; the thread closes it.
    _stop: Option<tokio::sync::oneshot::Sender<()>>,
    memory: Arc<CaptureMemory>,
    input_sink: Option<Arc<dyn crate::screen::core::host_backend::ScreenInputSink>>,
}

pub(crate) struct PortalSession;

impl PortalSession {
    /// Starts the portal negotiation on a thread of its own and returns at
    /// once.
    #[cfg(test)]
    pub(crate) fn start(options: PortalOptions) -> io::Result<PortalHandle> {
        let admission = super::host_backend::prepared::test_admission("portal-fixture");
        let key = SourceKey::new("portal-fixture", "portal")?;
        admission.register_prepared_source(key.clone())?;
        Self::prepare(options, Arc::new(CaptureMemory::new(key, admission)))
    }
    pub(crate) fn prepare(
        options: PortalOptions,
        memory: Arc<CaptureMemory>,
    ) -> io::Result<PortalHandle> {
        let state = Arc::new(Mutex::new(PortalState::Waiting));
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let shared = state.clone();
        let capture_memory = memory.clone();
        let input_sink = if options.remote_desktop {
            let inputs = super::portal_input::open_channel();
            let sink = inputs.input_sink(&memory);
            std::thread::Builder::new()
                .name("portal-session".into())
                .spawn(move || {
                    super::portal_input::thread_main(
                        &shared,
                        stopped,
                        options,
                        inputs,
                        capture_memory,
                    )
                })?;
            Some(sink)
        } else {
            std::thread::Builder::new()
                .name("portal-session".into())
                .spawn(move || thread_main(&shared, stopped, options, capture_memory))?;
            None
        };
        Ok(PortalHandle {
            state,
            _stop: Some(stop),
            memory,
            input_sink,
        })
    }
}

impl Drop for PortalHandle {
    fn drop(&mut self) {
        // The async owner receives this independently of the Parent control
        // thread and performs the serialized stream close before its join.
        self._stop.take();
    }
}

impl PortalHandle {
    fn lock(&self) -> MutexGuard<'_, PortalState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A handle in a given state, without a session behind it (tests).
    #[cfg(test)]
    pub(crate) fn detached(state: PortalState) -> Self {
        Self {
            state: Arc::new(Mutex::new(state)),
            _stop: None,
            memory: Arc::new(CaptureMemory::new(
                SourceKey::new("portal-fixture", "portal").unwrap(),
                super::host_backend::prepared::test_admission("portal-fixture"),
            )),
            input_sink: None,
        }
    }
}

pub(super) fn latest_for_state(state: &Mutex<PortalState>) -> Arc<LatestFrame> {
    let latest = Arc::new(LatestFrame::default());
    let active = !matches!(
        &*state.lock().unwrap_or_else(PoisonError::into_inner),
        PortalState::WaitingInactive
    );
    latest.set_initial_active(active);
    latest
}

pub(super) fn set(state: &Mutex<PortalState>, next: PortalState) {
    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
    if let PortalState::Streaming(latest) = &next {
        let active = !matches!(&*state, PortalState::WaitingInactive);
        if let Err(error) = latest.set_capture_active(active) {
            *state = PortalState::Failed(error.to_string());
            return;
        }
    }
    *state = next;
}

fn thread_main(
    state: &Arc<Mutex<PortalState>>,
    stopped: tokio::sync::oneshot::Receiver<()>,
    options: PortalOptions,
    memory: Arc<CaptureMemory>,
) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    match runtime {
        Ok(runtime) => runtime.block_on(run(state, stopped, options, memory)),
        Err(error) => set(
            state,
            PortalState::Failed(format!("portal_runtime: {error}")),
        ),
    }
}

/// Why the negotiation did not produce a stream.
pub(super) enum Refusal {
    Denied(String),
    Failed(String),
}

pub(super) fn refusal(error: ashpd::Error) -> Refusal {
    match error {
        ashpd::Error::Response(response) => Refusal::Denied(response.to_string()),
        other => Refusal::Failed(format!("portal: {other}")),
    }
}

/// What the dialog chose.
struct Chosen {
    identity: Arc<super::pipewire_stream::identity::StreamIdentity>,
    fd: std::os::fd::OwnedFd,
}

async fn run(
    state: &Arc<Mutex<PortalState>>,
    stopped: tokio::sync::oneshot::Receiver<()>,
    options: PortalOptions,
    memory: Arc<CaptureMemory>,
) {
    // Without PipeWire there is nothing to show: no dialog for nothing.
    let api = match pipewire_abi::load_named(options.pipewire_library) {
        Ok(api) => api,
        Err(error) => {
            eprintln!("screen host process: {error}");
            set(state, PortalState::Failed(PIPEWIRE_MISSING.into()));
            return;
        }
    };
    let connection = match connection::shared().await {
        Ok(connection) => connection,
        Err(error) => {
            set(state, PortalState::Failed(format!("portal: {error}")));
            return;
        }
    };
    register_host_app_once(&connection).await;
    let screencast = match ashpd::desktop::screencast::Screencast::with_connection(connection).await
    {
        Ok(screencast) => screencast,
        Err(error) => {
            set(state, PortalState::Failed(format!("portal: {error}")));
            return;
        }
    };
    let session = match screencast.create_session(Default::default()).await {
        Ok(session) => session,
        Err(error) => {
            set(state, PortalState::Failed(format!("portal: {error}")));
            return;
        }
    };
    use futures_util::future::{select, Either};
    use futures_util::StreamExt;
    let closed = match session.receive_closed().await {
        Ok(closed) => closed,
        Err(error) => {
            set(
                state,
                PortalState::Failed(format!("portal_closed_subscription: {error}")),
            );
            let _ = session.close().await;
            return;
        }
    };
    let mut closed = Box::pin(closed);
    let flow = Box::pin(negotiate(&screencast, &session));
    let stop = Box::pin(async {
        let _ = select(Box::pin(closed.next()), stopped).await;
    });
    match select(flow, stop).await {
        Either::Left((Ok(chosen), stop)) => {
            let latest = latest_for_state(state);
            let identity = chosen.identity;
            match PwStream::start(api, chosen.fd, identity.clone(), latest.clone(), memory) {
                Ok(stream) => {
                    set(state, PortalState::Streaming(latest.clone()));
                    let _ = select(stop, Box::pin(latest.wait_failure())).await;
                    latest.fail("portal_closed".into());
                    identity.invalidate();
                    drop(stream);
                }
                Err(error) => set(state, PortalState::Failed(error.to_string())),
            }
        }
        Either::Left((Err(Refusal::Denied(text)), _)) => set(state, PortalState::Denied(text)),
        Either::Left((Err(Refusal::Failed(text)), _)) => set(state, PortalState::Failed(text)),
        Either::Right(_) => set(state, PortalState::Failed("portal_closed".into())),
    }
    let _ = session.close().await;
}

/// Source selection, start, token storage and the PipeWire descriptor.
async fn negotiate(
    screencast: &ashpd::desktop::screencast::Screencast,
    session: &ashpd::desktop::Session<ashpd::desktop::screencast::Screencast>,
) -> Result<Chosen, Refusal> {
    use ashpd::desktop::screencast::OpenPipeWireRemoteOptions;
    use ashpd::desktop::PersistMode;
    let stored = portal_tokens::load();
    let token = stored
        .get(TokenKind::ScreenCast)
        .map(|stored| stored.token.clone());
    let select_options = source_binding::advertised_sources(screencast)
        .await
        .set_restore_token(token.as_deref())
        .set_persist_mode(PersistMode::ExplicitlyRevoked);
    let selected = screencast
        .select_sources(session, select_options)
        .await
        .map_err(refusal)?;
    selected.response().map_err(refusal)?;
    let streams = source_binding::start(screencast, session).await?;
    // The portal hands out a fresh token with every start; the old one is
    // replaced at once.
    if let Some(token) = streams.restore_token.as_deref() {
        if let Err(error) = portal_tokens::save(TokenKind::ScreenCast, token) {
            eprintln!("screen host process: portal token not stored: {error}");
        }
    }
    let identity = streams.selected(session, screencast.version())?;
    let fd = screencast
        .open_pipe_wire_remote(session, OpenPipeWireRemoteOptions::default())
        .await
        .map_err(refusal)?;
    Ok(Chosen { identity, fd })
}

/// Registers the program with the portal once per process; a portal before
/// 1.19.4 or an application id without a dot fails, which changes nothing
/// for the screen cast (A10).
pub(super) async fn register_host_app_once(connection: &ashpd::zbus::Connection) {
    static TRIED: AtomicBool = AtomicBool::new(false);
    if TRIED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Ok(app_id) = "smart-explorer".parse::<ashpd::AppID>() {
        let _ = ashpd::register_host_app_with_connection(connection.clone(), app_id).await;
    }
}

/// The screen source of a portal session.
pub(crate) struct PortalSource {
    handle: PortalHandle,
    size: FrameSize,
    had_frame: bool,
}

impl PortalSource {
    pub(crate) fn prepare(
        options: PortalOptions,
        key: SourceKey,
        admission: Arc<SessionAdmission>,
    ) -> io::Result<Self> {
        Ok(Self::with_handle(PortalSession::prepare(
            options,
            Arc::new(CaptureMemory::new(key, admission)),
        )?))
    }
    pub(crate) fn memory(&self) -> &Arc<CaptureMemory> {
        &self.handle.memory
    }
    pub(crate) fn check_preparation(&self) -> io::Result<()> {
        match &*self.handle.lock() {
            PortalState::Denied(reason) => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("portal_denied: {reason}"),
            )),
            PortalState::Failed(reason) => Err(io::Error::other(reason.clone())),
            PortalState::Streaming(latest) => latest.check_failure(),
            _ => Ok(()),
        }
    }
    /// Starts the portal session; never waits for the dialog.
    #[cfg(test)]
    pub(crate) fn open(options: PortalOptions) -> io::Result<Self> {
        Ok(Self::with_handle(PortalSession::start(options)?))
    }

    pub(crate) fn with_handle(handle: PortalHandle) -> Self {
        Self {
            handle,
            size: PORTAL_PROVISIONAL,
            had_frame: false,
        }
    }
}

impl ScreenSource for PortalSource {
    fn inject_input(
        &mut self,
        _backend: &dyn crate::screen::core::host_backend::ScreenHostBackend,
        reference_generation: u64,
        presented: bool,
        events: &[crate::screen::core::input_model::InputEvent],
    ) -> Result<(), crate::screen::core::wire_control::ControlBlock> {
        self.handle
            .input_sink
            .as_ref()
            .ok_or(crate::screen::core::wire_control::ControlBlock::ControlUnavailable)?
            .inject(events, reference_generation, presented)
    }
    fn input_sink(&self) -> Option<Arc<dyn crate::screen::core::host_backend::ScreenInputSink>> {
        self.handle.input_sink.clone()
    }
    fn capture_grant(&self) -> Option<CaptureGrant> {
        self.memory().current_grant()
    }
    fn reference_generation(&self) -> u64 {
        self.memory().generation().value()
    }
    fn frame_limits(&self) -> crate::screen::core::frame_budget::FrameLimits {
        crate::screen::core::frame_budget::FrameLimits::RESOURCE
    }

    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        let mut state = self.handle.lock();
        match &*state {
            PortalState::Waiting | PortalState::WaitingInactive => {
                *state = if active {
                    PortalState::Waiting
                } else {
                    PortalState::WaitingInactive
                };
                Ok(())
            }
            PortalState::Streaming(latest) => {
                latest.set_capture_active(active)?;
                self.had_frame = false;
                Ok(())
            }
            PortalState::Denied(reason) | PortalState::Failed(reason) => {
                Err(io::Error::new(io::ErrorKind::NotConnected, reason.clone()))
            }
        }
    }
    fn size(&self) -> FrameSize {
        self.size
    }

    fn capture(&mut self, _frame_id: u64, keyframe: bool) -> io::Result<SourceFrame> {
        #[cfg(test)]
        self.memory().activate_for_test()?;
        let latest = match &*self.handle.lock() {
            PortalState::Waiting | PortalState::WaitingInactive => {
                return Ok(SourceFrame::Paused(SessionState::WaitingForPortal))
            }
            PortalState::Streaming(latest) => latest.clone(),
            PortalState::Denied(text) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("portal_denied: {text}"),
                ))
            }
            PortalState::Failed(text) if text == PIPEWIRE_MISSING => {
                return Ok(SourceFrame::Paused(SessionState::NoDisplay))
            }
            PortalState::Failed(text) => return Err(io::Error::other(text.clone())),
        };
        match latest.take(keyframe) {
            Taken::Frame(image) => {
                self.size = image.size();
                self.had_frame = true;
                Ok(SourceFrame::Frame(image))
            }
            Taken::Unchanged if !self.had_frame => {
                Ok(SourceFrame::Paused(SessionState::WaitingForPortal))
            }
            Taken::Unchanged => Ok(SourceFrame::Unchanged),
            Taken::Failed(text) => Err(io::Error::other(text)),
        }
    }
}

#[cfg(test)]
#[path = "portal_tests.rs"]
mod tests;
