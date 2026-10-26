//! Shared desktop/Android owner. One latest candidate, two real raw owners.
use super::viewer_images;
use crate::login::core::viewer_guard::ViewerGuard;
use crate::screen::core::{capture_image::CapturedImage, image_admission::ImageAdmission};
use crate::{
    daemon::{LoginClose, LoginCommand, LoginSessionView, LoginViewer},
    login::{FrameToken, GatewayTarget},
    screen::core::input_model::{encode_batch, InputEvent, PressedInputs, TouchPhase},
};
use serde::Serialize;
use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};
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
    pub fn presented(&self, token: &FrameToken, epoch: u64) -> bool {
        let Ok(mut state) = self.shared.state.lock() else {
            return false;
        };
        let accepted = state.guard.presented(token, epoch);
        refresh(&mut state);
        accepted
    }
    pub fn invalidate_presentation(&self) {
        self.invalidate_presentation_at(None);
    }
    /// A late renderer failure may revoke only the epoch that owned its pixels.
    pub(crate) fn renderer_failed(&self, epoch: u64) {
        self.invalidate_presentation_at(Some(epoch));
    }
    fn invalidate_presentation_at(&self, epoch: Option<u64>) {
        let mut state = match self.shared.state.lock() {
            Ok(state) => state,
            Err(error) => {
                drop(error);
                self.close();
                return;
            }
        };
        if epoch.is_some_and(|epoch| epoch != state.guard.epoch) {
            return;
        }
        self.shared.release.store(true, Ordering::SeqCst);
        state.guard.invalidate();
        let interrupt = must_interrupt(&self.shared, &state);
        refresh(&mut state);
        drop(state);
        if interrupt {
            self.close();
        }
    }
    pub fn focus(&self, focused: bool, control: bool) {
        let mut interrupt = false;
        if let Ok(mut state) = self.shared.state.lock() {
            if state.guard.focused != focused || state.guard.control != control {
                state.guard.invalidate();
            }
            if (state.guard.focused && !focused) || (state.guard.control && !control) {
                self.shared.release.store(true, Ordering::SeqCst);
                interrupt = must_interrupt(&self.shared, &state);
            }
            state.guard.focused = focused;
            state.guard.control = control;
            refresh(&mut state);
        }
        if interrupt {
            self.close();
        }
    }
    pub fn input(
        &self,
        token: FrameToken,
        epoch: u64,
        events: Vec<InputEvent>,
    ) -> Result<(), String> {
        encode_batch(&events).map_err(invalid)?;
        if events
            .iter()
            .any(|event| matches!(event, InputEvent::Action { .. }))
        {
            return Err("Android actions are unavailable at login".into());
        }
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| "login viewer state is unavailable")?;
        let allowed = state.guard.epoch == epoch && state.guard.input_ready(&token);
        drop(state);
        if !allowed {
            return Err("login input requires the focused, actually drawn current frame".into());
        }
        self.queue(Command::Input(token, epoch, events))
    }
    pub fn secure_attention(&self, token: FrameToken, epoch: u64) -> Result<(), String> {
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| "login viewer state is unavailable")?;
        if state.guard.epoch != epoch
            || !state.guard.action_ready(&token)
            || !state
                .snapshot
                .view
                .as_ref()
                .and_then(|view| view.access.as_ref())
                .is_some_and(|access| {
                    access.rights.sas && access.rights.screen_control && access.rights.login
                })
        {
            return Err("secure attention requires the current drawn frame and explicit administrative SAS/control rights".into());
        }
        drop(state);
        self.queue(Command::Sas(token, epoch))
    }
    fn queue(&self, command: Command) -> Result<(), String> {
        if self.shared.stopped.load(Ordering::SeqCst) {
            return Err("login viewer ended".into());
        }
        if self.commands.try_send(command).is_err() {
            self.close();
            return Err("login input queue is full or closed; session ended without retry".into());
        }
        Ok(())
    }
    pub fn release(&self) {
        self.shared.release.store(true, Ordering::SeqCst);
        let mut interrupt = false;
        if let Ok(mut state) = self.shared.state.lock() {
            state.guard.focused = false;
            state.guard.control = false;
            state.guard.invalidate();
            interrupt = must_interrupt(&self.shared, &state);
            refresh(&mut state);
        }
        if interrupt {
            self.close();
        }
    }
    pub fn close(&self) {
        if self.shared.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Ok(mut state) = self.shared.state.lock() {
            state.guard.close();
            state.frame = None;
            state.snapshot.state = "stopping".into();
            refresh(&mut state);
            if let Some(close) = &state.close {
                close.close();
            }
        }
        self.shared.changed.notify_all();
    }
    /// Legacy JNI copying has no admitted destination owner. L7 replaces
    /// this entrypoint with the bitmap-ticket transfer of a whole ViewerFrame.
    /// No raw copy may escape through the old DirectBuffer/IntArray path.
    pub fn pull(&self, _dst: &mut [u8], _wait: Duration) -> i32 {
        if self.shared.stopped.load(Ordering::SeqCst) {
            -2
        } else {
            -4
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
fn run(
    shared: &Shared,
    mut viewer: LoginViewer,
    input: mpsc::Receiver<Command>,
) -> Result<(), String> {
    if shared.stopped.load(Ordering::SeqCst) {
        viewer.close();
        return Ok(());
    }
    if shared
        .state
        .lock()
        .map_err(|_| "login viewer state is unavailable")?
        .visible
        .elapsed()
        > Duration::from_secs(2)
    {
        viewer.close();
        return Err("login viewer owner disappeared during connection setup".into());
    }
    let started = exchange(shared, &mut viewer, LoginCommand::Start)?;
    require_state(&started, "ready")?;
    adopt(shared, &viewer)?;
    let mut frame_id = 1u64;
    let mut captured = Instant::now() - Duration::from_secs(1);
    let mut had_input = false;
    loop {
        if shared.stopped.load(Ordering::SeqCst) {
            viewer.close();
            return Ok(());
        }
        if shared
            .state
            .lock()
            .map_err(|_| "login viewer state is unavailable")?
            .visible
            .elapsed()
            > Duration::from_secs(2)
        {
            viewer.close();
            return Err("login viewer is no longer visibly owned".into());
        }
        if shared.release.swap(false, Ordering::SeqCst) && had_input {
            exchange(shared, &mut viewer, LoginCommand::ReleaseInputs)?;
            had_input = false;
            if let Ok(mut state) = shared.state.lock() {
                state.pressed.release_all();
            }
            adopt(shared, &viewer)?;
        }
        if let Some(end) = viewer.poll_end()? {
            return Err(end.detail.unwrap_or(end.code));
        }
        match input.recv_timeout(Duration::from_millis(10)) {
            Ok(command) => {
                let state = shared
                    .state
                    .lock()
                    .map_err(|_| "login viewer state is unavailable")?;
                let allowed = command_allowed(&state, &command);
                drop(state);
                if !allowed {
                    if let Ok(mut state) = shared.state.lock() {
                        state.snapshot.notice = Some("Eingabe wegen geänderter Bild-, Fokus- oder Rechtebindung verworfen; keine automatische Wiederholung.".into());
                    }
                    shared.release.store(true, Ordering::SeqCst);
                    continue;
                }
                match command {
                    Command::Input(_, _, events) => {
                        had_input = true;
                        if let Ok(mut state) = shared.state.lock() {
                            for event in &events {
                                if !matches!(
                                    event,
                                    InputEvent::Key { down: false, .. }
                                        | InputEvent::Button { down: false, .. }
                                        | InputEvent::Touch {
                                            phase: TouchPhase::End | TouchPhase::Cancel,
                                            ..
                                        }
                                ) {
                                    state.pressed.observe(event);
                                }
                            }
                        }
                        let attempted = events.clone();
                        let result = exchange(shared, &mut viewer, LoginCommand::Input { events })?;
                        require_state(&result, "input_applied")?;
                        if let Ok(mut state) = shared.state.lock() {
                            for event in &attempted {
                                state.pressed.observe(event);
                            }
                        }
                    }
                    Command::Sas(_, _) => {
                        let result = exchange(shared, &mut viewer, LoginCommand::SecureAttention)?;
                        let text = result.reply.packet.as_ref().map(|packet| {
                            match packet.reply.reason.as_deref() {
                                Some(reason) => format!(
                                    "Sichere Aufmerksamkeit: {} · {reason}",
                                    packet.reply.state
                                ),
                                None => format!("Sichere Aufmerksamkeit: {}", packet.reply.state),
                            }
                        });
                        if let Ok(mut state) = shared.state.lock() {
                            state.snapshot.notice = text;
                        }
                    }
                }
                adopt(shared, &viewer)?;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                viewer.close();
                return Ok(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if shared.release.load(Ordering::SeqCst) {
            continue;
        }
        if captured.elapsed() >= Duration::from_millis(100) {
            if !viewer_images::can_capture(&shared.images).map_err(invalid)? {
                // Latest has not left this owner and may be superseded. Images
                // already transferred to UI/JNI continue consuming their slots.
                if let Ok(mut state) = shared.state.lock() {
                    state.frame.take();
                }
                if !viewer_images::can_capture(&shared.images).map_err(invalid)? {
                    continue;
                }
            }
            let frame = exchange(shared, &mut viewer, LoginCommand::Capture { frame_id })?;
            adopt(shared, &viewer)?;
            if let Some(image) = frame.image {
                let packet = frame
                    .reply
                    .packet
                    .as_ref()
                    .ok_or_else(|| "login capture has no bounded metadata".to_string())?;
                let mut state = shared
                    .state
                    .lock()
                    .map_err(|_| "login viewer state is unavailable")?;
                if shared.stopped.load(Ordering::SeqCst) {
                    state.guard.close();
                    state.frame = None;
                    return Ok(());
                }
                let token = state
                    .guard
                    .frame(frame_id, packet.reply.width, packet.reply.height)
                    .map_err(invalid)?;
                state.frame = Some(ViewerFrame { token, image });
                refresh(&mut state);
                shared.changed.notify_all();
            } else if !matches!(
                frame.reply.view.state.as_str(),
                "unchanged" | "ready" | "pixels"
            ) {
                return Err(frame
                    .reply
                    .packet
                    .as_ref()
                    .and_then(|packet| packet.reply.reason.clone())
                    .unwrap_or_else(|| {
                        format!("login capture ended: {}", frame.reply.view.state)
                    }));
            }
            frame_id = frame_id
                .checked_add(1)
                .filter(|id| *id <= i64::MAX as u64)
                .ok_or_else(|| "login frame sequence exhausted".to_string())?;
            captured = Instant::now();
        }
    }
}
fn must_interrupt(shared: &Shared, state: &State) -> bool {
    shared.input_flight.load(Ordering::SeqCst)
        || (shared.operating.load(Ordering::SeqCst) && !state.pressed.is_empty())
}
fn command_allowed(state: &State, command: &Command) -> bool {
    let rights = state
        .snapshot
        .view
        .as_ref()
        .and_then(|view| view.access.as_ref())
        .map(|access| &access.rights);
    match command {
        Command::Input(token, epoch, _) => {
            state.guard.epoch == *epoch
                && state.guard.input_ready(token)
                && rights.is_some_and(|rights| {
                    rights.screen_control && rights.screen_view && rights.login
                })
        }
        Command::Sas(token, epoch) => {
            state.guard.epoch == *epoch
                && state.guard.action_ready(token)
                && rights.is_some_and(|rights| rights.sas && rights.screen_control && rights.login)
        }
    }
}
fn begin_exchange(shared: &Shared, command: &LoginCommand) {
    shared.operating.store(true, Ordering::SeqCst);
    shared.input_flight.store(
        matches!(
            command,
            LoginCommand::Input { .. } | LoginCommand::SecureAttention
        ),
        Ordering::SeqCst,
    );
}
fn exchange(
    shared: &Shared,
    viewer: &mut LoginViewer,
    command: LoginCommand,
) -> Result<crate::daemon::LoginFrame, String> {
    struct InFlight<'a>(&'a Shared);
    impl Drop for InFlight<'_> {
        fn drop(&mut self) {
            self.0.operating.store(false, Ordering::SeqCst);
            self.0.input_flight.store(false, Ordering::SeqCst);
        }
    }
    begin_exchange(shared, &command);
    let _flight = InFlight(shared);
    if shared.stopped.load(Ordering::SeqCst) {
        viewer.close();
        return Err("login viewer owner already ended".into());
    }
    let has_pressed = !shared
        .state
        .lock()
        .map_err(|_| "login viewer state is unavailable")?
        .pressed
        .is_empty();
    if !matches!(command, LoginCommand::ReleaseInputs)
        && shared.release.load(Ordering::SeqCst)
        && (shared.input_flight.load(Ordering::SeqCst) || has_pressed)
    {
        viewer.close();
        return Err(
            "login display/focus changed before an operation was sent; owner closed without retry"
                .into(),
        );
    }
    viewer.command(command)
}
fn require_state(frame: &crate::daemon::LoginFrame, expected: &str) -> Result<(), String> {
    let reply = frame
        .reply
        .packet
        .as_ref()
        .ok_or_else(|| "login operation has no broker reply".to_string())?;
    if reply.reply.state != expected {
        return Err(format!(
            "{}: {}",
            reply.reply.state,
            reply
                .reply
                .reason
                .as_deref()
                .unwrap_or("login operation was not admitted")
        ));
    }
    Ok(())
}

/// Controlled renderer events use the real guard, queue check and exchange flags.
/// No IPC command, privileged input or SAS is executed by this fixture.
#[cfg(test)]
pub(crate) struct RendererFixture {
    pub session: Arc<ViewerSession>,
    commands: mpsc::Receiver<Command>,
}
#[cfg(test)]
impl RendererFixture {
    pub fn new(view: LoginSessionView, images: Arc<ImageAdmission>) -> Self {
        let mut guard = ViewerGuard::default();
        guard
            .adopt(
                view.challenge.as_ref().unwrap(),
                view.access.as_ref().unwrap(),
            )
            .unwrap();
        let shared = Arc::new(Shared {
            images,
            state: Mutex::new(State {
                snapshot: ViewerSnapshot {
                    state: "ready".into(),
                    reason: None,
                    view: Some(view),
                    frame: None,
                    input_ready: false,
                    control: false,
                    focused: false,
                    presented_frame: None,
                    presentation_epoch: 0,
                    notice: None,
                },
                guard,
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
        let (commands, pending) = mpsc::sync_channel(8);
        Self {
            session: Arc::new(ViewerSession { shared, commands }),
            commands: pending,
        }
    }
    pub fn frame(&self, id: u64, width: u32, height: u32) -> FrameToken {
        let mut state = self.session.shared.state.lock().unwrap();
        let token = state.guard.frame(id, width, height).unwrap();
        refresh(&mut state);
        token
    }
    pub fn queue_image(&self, id: u64, image: CapturedImage) -> FrameToken {
        let size = image.size();
        let token = self.frame(id, size.width, size.height);
        self.session.shared.state.lock().unwrap().frame = Some(ViewerFrame {
            token: token.clone(),
            image,
        });
        token
    }
    pub fn queued_allowed(&self) -> Option<bool> {
        let command = self.commands.try_recv().ok()?;
        Some(command_allowed(
            &self.session.shared.state.lock().unwrap(),
            &command,
        ))
    }
    pub fn release_pending(&self) -> bool {
        self.session.shared.release.load(Ordering::SeqCst)
    }
    /// Explicit event standing for completed release handling by the owner.
    pub fn complete_release(&self) {
        self.session
            .shared
            .state
            .lock()
            .unwrap()
            .pressed
            .release_all();
        self.session.shared.release.store(false, Ordering::SeqCst);
    }
    pub fn begin_queued_exchange(&self) -> bool {
        let Ok(command) = self.commands.try_recv() else {
            return false;
        };
        if !command_allowed(&self.session.shared.state.lock().unwrap(), &command) {
            return false;
        }
        let command = match command {
            Command::Input(_, _, events) => LoginCommand::Input { events },
            Command::Sas(_, _) => LoginCommand::SecureAttention,
        };
        begin_exchange(&self.session.shared, &command);
        true
    }
}
