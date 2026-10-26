//! Shared desktop/Android owner. One latest candidate, two real raw owners.
use super::viewer_images;
use crate::login::core::viewer_guard::ViewerGuard;
use crate::screen::core::{capture_image::CapturedImage, image_admission::ImageAdmission};
use crate::{
    daemon::{LoginClose, LoginSessionView, LoginViewer},
    login::{FrameToken, GatewayTarget},
    screen::core::input_model::{InputEvent, PressedInputs},
};
use serde::Serialize;
#[cfg(test)]
use std::time::Duration;
use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
        Arc, Condvar, Mutex,
    },
    time::Instant,
};
#[path = "viewer_commands.rs"]
mod commands;
#[path = "viewer_transport.rs"]
mod transport;
use transport::run;
#[cfg(test)]
#[path = "viewer_renderer_fixture.rs"]
mod renderer_fixture;
#[cfg(test)]
pub(crate) use renderer_fixture::RendererFixture;

#[derive(Clone, Serialize)]
pub(crate) struct ViewerSnapshot {
    pub state: String,
    pub reason: Option<String>,
    pub view: Option<LoginSessionView>,
    pub frame: Option<FrameToken>,
    pub input_ready: bool,
    pub control: bool,
    pub focused: bool,
    pub presented_frame: Option<FrameToken>,
    pub presentation_epoch: u64,
    pub notice: Option<String>,
}
pub(crate) struct ViewerFrame {
    pub token: FrameToken,
    pub image: CapturedImage,
}
struct State {
    snapshot: ViewerSnapshot,
    guard: ViewerGuard,
    frame: Option<ViewerFrame>,
    close: Option<LoginClose>,
    visible: Instant,
    pressed: PressedInputs,
}
struct Shared {
    images: Arc<ImageAdmission>,
    state: Mutex<State>,
    changed: Condvar,
    stopped: AtomicBool,
    release: AtomicBool,
    operating: AtomicBool,
    input_flight: AtomicBool,
}
enum Command {
    Input(FrameToken, u64, Vec<InputEvent>),
    Sas(FrameToken, u64),
}
pub(crate) struct ViewerSession {
    shared: Arc<Shared>,
    commands: SyncSender<Command>,
}
fn invalid(reason: impl ToString) -> String {
    reason.to_string()
}
impl ViewerSession {
    pub fn start(
        target: GatewayTarget,
        user: String,
        pin_acknowledged: bool,
    ) -> io::Result<Arc<Self>> {
        let images = viewer_images::admission()?;
        Self::start_with_images(target, user, pin_acknowledged, images)
    }
    pub(crate) fn start_with_images(
        target: GatewayTarget,
        user: String,
        pin_acknowledged: bool,
        images: Arc<ImageAdmission>,
    ) -> io::Result<Arc<Self>> {
        target.validate()?;
        if !pin_acknowledged {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "explicit administrative machine pin acknowledgement is required",
            ));
        }
        Self::spawn(images, move |shared| {
            LoginViewer::open_owned_with_images(
                target,
                user,
                true,
                shared.images.clone(),
                |close| {
                    if let Ok(mut state) = shared.state.lock() {
                        if shared.stopped.load(Ordering::SeqCst) {
                            close.close();
                        }
                        state.close = Some(close);
                    }
                },
            )
        })
    }
    fn spawn(
        images: Arc<ImageAdmission>,
        open: impl FnOnce(&Shared) -> Result<LoginViewer, String> + Send + 'static,
    ) -> io::Result<Arc<Self>> {
        let shared = Arc::new(Shared {
            images,
            state: Mutex::new(State {
                snapshot: ViewerSnapshot {
                    state: "connecting".into(),
                    reason: None,
                    view: None,
                    frame: None,
                    input_ready: false,
                    control: false,
                    focused: false,
                    presented_frame: None,
                    presentation_epoch: 0,
                    notice: None,
                },
                guard: ViewerGuard::default(),
                frame: None,
                close: None,
                visible: Instant::now(),
                pressed: PressedInputs::default(),
            }),
            changed: Condvar::new(),
            stopped: AtomicBool::new(false),
            release: AtomicBool::new(false),
            operating: AtomicBool::new(false),
            input_flight: AtomicBool::new(false),
        });
        let (commands, input) = mpsc::sync_channel(8);
        let session = Arc::new(Self {
            shared: shared.clone(),
            commands,
        });
        std::thread::Builder::new()
            .name("login-viewer-owner".into())
            .spawn(move || {
                let result = open(&shared).and_then(|viewer| run(&shared, viewer, input));
                if let Ok(mut state) = shared.state.lock() {
                    state.guard.close();
                    state.frame = None;
                    state.close.take();
                    state.snapshot.frame = None;
                    state.snapshot.input_ready = false;
                    state.snapshot.state = "ended".into();
                    state.snapshot.reason = result.err().map(|reason| {
                        if reason.contains("permission_missing:") {
                            format!("Recht fehlt · {reason}")
                        } else if reason.contains("installation_required:") {
                            format!("Administrative Installation erforderlich · {reason}")
                        } else if reason.contains("unavailable:") {
                            format!("Anmeldung im aktuellen Hostzustand nicht verfügbar · {reason}")
                        } else {
                            reason
                        }
                    });
                    refresh(&mut state);
                }
                shared.stopped.store(true, Ordering::SeqCst);
                shared.changed.notify_all();
            })?;
        Ok(session)
    }
    #[cfg(test)]
    pub(crate) fn from_fixture_viewer(
        viewer: LoginViewer,
        images: Arc<ImageAdmission>,
    ) -> io::Result<Arc<Self>> {
        Self::spawn(images, move |shared| {
            if let Ok(mut state) = shared.state.lock() {
                state.close = Some(viewer.control());
            }
            Ok(viewer)
        })
    }
    #[cfg(test)]
    pub(crate) fn wait_fixture_frame(&self, wait: Duration) -> Option<ViewerFrame> {
        let state = self.shared.state.lock().ok()?;
        let (mut state, _) = self
            .shared
            .changed
            .wait_timeout_while(state, wait, |state| {
                state.frame.is_none() && !self.shared.stopped.load(Ordering::SeqCst)
            })
            .ok()?;
        state.frame.take()
    }
    #[cfg(test)]
    pub(crate) fn wait_fixture_ready(&self, wait: Duration) -> bool {
        let Ok(state) = self.shared.state.lock() else {
            return false;
        };
        self.shared
            .changed
            .wait_timeout_while(state, wait, |state| {
                state.frame.is_none() && !self.shared.stopped.load(Ordering::SeqCst)
            })
            .is_ok_and(|(state, _)| state.frame.is_some())
    }
    pub fn snapshot(&self) -> ViewerSnapshot {
        self.shared
            .state
            .lock()
            .map(|state| state.snapshot.clone())
            .unwrap_or_else(|_| ViewerSnapshot {
                state: "ended".into(),
                reason: Some("login viewer state is unavailable".into()),
                view: None,
                frame: None,
                input_ready: false,
                control: false,
                focused: false,
                presented_frame: None,
                presentation_epoch: u64::MAX,
                notice: None,
            })
    }
    pub fn take_frame(&self) -> Option<ViewerFrame> {
        let mut state = self.shared.state.lock().ok()?;
        if self.shared.stopped.load(Ordering::SeqCst) {
            state.frame = None;
            return None;
        }
        state.frame.take()
    }
    pub fn visible(&self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.visible = Instant::now();
        }
    }
}

impl Drop for ViewerSession {
    fn drop(&mut self) {
        self.close();
    }
}
fn refresh(state: &mut State) {
    state.snapshot.frame = state.guard.latest.clone();
    state.snapshot.control = state.guard.control;
    state.snapshot.focused = state.guard.focused;
    state.snapshot.presented_frame = state.guard.presented.clone();
    state.snapshot.presentation_epoch = state.guard.epoch;
    state.snapshot.input_ready = state
        .guard
        .presented
        .as_ref()
        .is_some_and(|token| state.guard.input_ready(token));
    state.snapshot.input_ready &= state
        .snapshot
        .view
        .as_ref()
        .and_then(|view| view.access.as_ref())
        .is_some_and(|access| access.rights.screen_control);
}
fn adopt(shared: &Shared, viewer: &LoginViewer) -> Result<(), String> {
    let mut state = shared
        .state
        .lock()
        .map_err(|_| "login viewer state is unavailable")?;
    if shared.stopped.load(Ordering::SeqCst) {
        state.guard.close();
        state.frame = None;
        refresh(&mut state);
        return Ok(());
    }
    let view = viewer.view();
    if let Some(challenge) = &view.challenge {
        let access = view.access.as_ref().ok_or_else(|| {
            "login frame has no authenticated administrative access report".to_string()
        })?;
        state.guard.adopt(challenge, access).map_err(invalid)?;
    } else {
        state.guard.close();
    }
    state.snapshot.view = Some(view.clone());
    state.snapshot.state = view.state.clone();
    refresh(&mut state);
    Ok(())
}
