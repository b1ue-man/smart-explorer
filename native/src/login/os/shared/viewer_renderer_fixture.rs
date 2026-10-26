//! Controlled renderer receipts retain the real viewer command and epoch checks.
use super::commands::command_allowed;
use super::transport::begin_exchange;
use super::*;
use crate::daemon::LoginCommand;

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
