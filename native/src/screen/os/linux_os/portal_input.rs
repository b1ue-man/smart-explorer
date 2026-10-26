//! Input on a Wayland host (SCR-M6, C5): a session that may control runs as
//! an `org.freedesktop.portal.RemoteDesktop` session with the screen cast in
//! it, so the host user grants watching and controlling in one dialog;
//! devices (keyboard and pointer) are chosen before the sources
//! (`SelectDevices` before `SelectSources`, then `Start`). Input goes
//! through the same session: absolute pointer positions on the cast stream,
//! buttons as Linux button codes, the wheel in discrete steps, keys as
//! Linux key codes, text as keysyms. Without the dialog's consent, on
//! desktops without the RemoteDesktop portal (wlroots) or before the stream
//! runs, input is refused with `control_unavailable`. No event is logged.
//!
//! Assumption: the `ashpd` 0.13 names and option types are those of its
//! documentation (docs.rs, read 2026-10-19: `create_session(options)`,
//! `select_devices(session, SelectDevicesOptions)`, `start(session, None,
//! StartOptions)`, `notify_*(…, options)`); they could not be built here.
use std::sync::{Arc, Mutex, PoisonError};

use ashpd::desktop::remote_desktop::{
    Axis, DeviceType, KeyState, RemoteDesktop, SelectDevicesOptions,
};
use ashpd::desktop::screencast::{
    CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
};
use ashpd::desktop::{PersistMode, Session};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use super::pipewire_abi;
use super::pipewire_stream::{LatestFrame, PwStream};
use super::pipewire_stream::identity::StreamIdentity;
use super::portal_session::source_binding;
use super::portal_session::{
    refusal, register_host_app_once, set, PortalOptions, PortalState, Refusal, PIPEWIRE_MISSING,
};
use super::portal_tokens::{self, TokenKind};
use super::x11_keymap::keysym_of;
use crate::screen::core::input_model::{InputEvent, PointerButton, TouchPhase};
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::os::input_keys::{evdev_code, SasGuard};

type Batch = Vec<InputEvent>;

/// The input of this host process's session; `None` without one.
static INPUT: Mutex<Option<Arc<InputLease>>> = Mutex::new(None);

struct InputLease {
    sender: UnboundedSender<Batch>,
    binding: Mutex<Option<Arc<StreamIdentity>>>,
}

pub(super) struct SessionInput {
    receiver: UnboundedReceiver<Batch>,
    lease: Arc<InputLease>,
}

impl Drop for SessionInput {
    fn drop(&mut self) {
        if let Some(binding) = self.lease.binding.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            binding.invalidate();
        }
    }
}

/// The channel of a new session; its sender becomes this process's input.
pub(super) fn open_channel() -> SessionInput {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let lease = Arc::new(InputLease { sender, binding: Mutex::new(None) });
    let old = INPUT.lock().unwrap_or_else(PoisonError::into_inner).replace(lease.clone());
    if let Some(old) = old {
        if let Some(binding) = old.binding.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            binding.invalidate();
        }
    }
    SessionInput { receiver, lease }
}

/// Hands `events` to the portal session, or names why not.
pub(crate) fn inject(events: &[InputEvent]) -> Result<(), ControlBlock> {
    let lease = INPUT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or(ControlBlock::ControlUnavailable)?;
    let binding = lease.binding.lock().unwrap_or_else(PoisonError::into_inner);
    if !binding.as_ref().is_some_and(|binding| binding.connected()) {
        return Err(ControlBlock::ControlUnavailable);
    }
    if binding.as_ref().is_some_and(|binding| binding.logical_size.is_none())
        && events.iter().any(|event| matches!(event, InputEvent::PointerAbs { .. } | InputEvent::Touch { .. })) {
        return Err(ControlBlock::ControlUnavailable);
    }
    lease.sender
        .send(events.to_vec())
        .map_err(|_| ControlBlock::ControlUnavailable)
}

/// The thread of the session: its own current-thread runtime.
pub(super) fn thread_main(
    state: &Arc<Mutex<PortalState>>,
    stopped: tokio::sync::oneshot::Receiver<()>,
    options: PortalOptions,
    inputs: SessionInput,
) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    match runtime {
        Ok(runtime) => runtime.block_on(run(state, stopped, options, inputs)),
        Err(error) => set(
            state,
            PortalState::Failed(format!("portal_runtime: {error}")),
        ),
    }
}

/// What the dialog granted.
struct Granted {
    identity: Arc<StreamIdentity>,
    fd: std::os::fd::OwnedFd,
}

async fn run(
    state: &Arc<Mutex<PortalState>>,
    stopped: tokio::sync::oneshot::Receiver<()>,
    options: PortalOptions,
    mut inputs: SessionInput,
) {
    use futures_util::future::{select, Either};
    let api = match pipewire_abi::load_named(options.pipewire_library) {
        Ok(api) => api,
        Err(error) => {
            eprintln!("screen host process: {error}");
            set(state, PortalState::Failed(PIPEWIRE_MISSING.into()));
            return;
        }
    };
    let opened = async {
        let connection = super::portal_session::connection::shared().await?;
        register_host_app_once(&connection).await;
        let remote = RemoteDesktop::with_connection(connection.clone()).await?;
        let screencast = Screencast::with_connection(connection).await?;
        let session = remote.create_session(Default::default()).await?;
        Ok::<_, ashpd::Error>((remote, screencast, session))
    };
    let (remote, screencast, session) = match opened.await {
        Ok(opened) => opened,
        Err(error) => {
            set(state, PortalState::Failed(format!("portal: {error}")));
            return;
        }
    };
    use futures_util::StreamExt;
    let closed = match session.receive_closed().await {
        Ok(closed) => closed,
        Err(error) => {
            set(state, PortalState::Failed(format!("portal_closed_subscription: {error}")));
            let _ = session.close().await;
            return;
        }
    };
    let mut closed = Box::pin(closed);
    let mut stop = Box::pin(async { let _ = select(Box::pin(closed.next()), stopped).await; });
    let flow = Box::pin(negotiate(&remote, &screencast, &session));
    match select(flow, stop.as_mut()).await {
        Either::Left((Ok(granted), _)) => {
            let latest = Arc::new(LatestFrame::default());
            let Granted { identity, fd } = granted;
            let path = identity.session.clone();
            // A replaced input channel cannot attach a grant that arrived after replacement.
            let attached = {
                let current = INPUT.lock().unwrap_or_else(PoisonError::into_inner);
                let owned = current.as_ref().is_some_and(|current| Arc::ptr_eq(current, &inputs.lease));
                if owned {
                    *inputs.lease.binding.lock().unwrap_or_else(PoisonError::into_inner) = Some(identity.clone());
                }
                owned
            };
            if !attached {
                identity.invalidate();
                let _ = session.close().await;
                return;
            }
            match PwStream::start(api, fd, identity.clone(), latest.clone()) {
                Ok(stream) => {
                    set(state, PortalState::Streaming(latest.clone()));
                    let mut sas = SasGuard::new();
                    loop {
                        let next = Box::pin(inputs.receiver.recv());
                        // Session closure wins over queued input and can cancel a batch mid-await.
                        match select(stop.as_mut(), next).await {
                            Either::Right((Some(batch), _)) => {
                                let sent = Box::pin(apply(&remote, &session, &identity, &path, &batch, &mut sas));
                                if matches!(select(stop.as_mut(), sent).await, Either::Left(_)) {
                                    identity.invalidate();
                                    break;
                                }
                            }
                            Either::Right((None, _)) => break,
                            Either::Left(_) => {
                                identity.invalidate();
                                latest.fail("portal_closed".into());
                                break;
                            }
                        }
                    }
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

/// Devices, sources, start, token and the PipeWire descriptor.
async fn negotiate(
    remote: &RemoteDesktop,
    screencast: &Screencast,
    session: &Session<RemoteDesktop>,
) -> Result<Granted, Refusal> {
    let stored = portal_tokens::load();
    let token = stored
        .get(TokenKind::RemoteDesktop)
        .map(|stored| stored.token.clone());
    let devices = SelectDevicesOptions::default()
        .set_devices(DeviceType::Keyboard | DeviceType::Pointer)
        .set_restore_token(token.as_deref())
        .set_persist_mode(PersistMode::ExplicitlyRevoked);
    remote
        .select_devices(session, devices)
        .await
        .map_err(refusal)?
        .response()
        .map_err(refusal)?;
    let sources = SelectSourcesOptions::default()
        .set_cursor_mode(CursorMode::Embedded)
        .set_sources(SourceType::Monitor | SourceType::Window)
        .set_multiple(false);
    screencast
        .select_sources(session, sources)
        .await
        .map_err(refusal)?
        .response()
        .map_err(refusal)?;
    let started = source_binding::start(remote, session).await?;
    if let Some(token) = started.restore_token.as_deref() {
        if let Err(error) = portal_tokens::save(TokenKind::RemoteDesktop, token) {
            eprintln!("screen host process: portal token not stored: {error}");
        }
    }
    let wanted = DeviceType::Keyboard | DeviceType::Pointer;
    if started.devices.unwrap_or(0) & wanted.bits() != wanted.bits() {
        return Err(Refusal::Denied("remote_desktop_devices_refused".into()));
    }
    let identity = started.selected(session, screencast.version())?;
    let fd = screencast
        .open_pipe_wire_remote(session, OpenPipeWireRemoteOptions::default())
        .await
        .map_err(refusal)?;
    Ok(Granted { identity, fd })
}

/// Linux button code of `button` (`BTN_LEFT` …).
fn button_code(button: PointerButton) -> i32 {
    match button {
        PointerButton::Left => 0x110,
        PointerButton::Right => 0x111,
        PointerButton::Middle => 0x112,
        PointerButton::Back => 0x113,
        PointerButton::Forward => 0x114,
    }
}

fn key_state(down: bool) -> KeyState {
    if down {
        KeyState::Pressed
    } else {
        KeyState::Released
    }
}

/// The stream position of `x`, `y` (0–65535 across the stream).
pub(super) fn stream_point(size: (f64, f64), x: u16, y: u16) -> (f64, f64) {
    let scale = |pos: u16, length: f64| f64::from(pos) / 65_535.0 * (length - 1.0).max(0.0);
    (scale(x, size.0), scale(y, size.1))
}

/// Sends one batch; a failing call drops the rest of the batch.
async fn apply(
    remote: &RemoteDesktop,
    session: &Session<RemoteDesktop>,
    granted: &StreamIdentity,
    session_path: &str,
    batch: &[InputEvent],
    sas: &mut SasGuard,
) {
    for event in batch {
        if !granted.belongs_to(session_path) { return; }
        let sent = match event {
            InputEvent::PointerAbs { x, y } | InputEvent::Touch { id: 0, x, y, .. } => {
                // Missing logical size is not permission to use pixel/global-display coordinates.
                let Some((width, height)) = granted.logical_size else { return; };
                let (px, py) = stream_point((f64::from(width), f64::from(height)), *x, *y);
                let moved = remote
                    .notify_pointer_motion_absolute(
                        session,
                        granted.node_id,
                        px,
                        py,
                        Default::default(),
                    )
                    .await;
                match event {
                    InputEvent::Touch { phase, .. } => match phase {
                        TouchPhase::Start | TouchPhase::End | TouchPhase::Cancel => {
                            let down = *phase == TouchPhase::Start;
                            let button = remote
                                .notify_pointer_button(
                                    session,
                                    0x110,
                                    key_state(down),
                                    Default::default(),
                                )
                                .await;
                            moved.and(button)
                        }
                        TouchPhase::Move => moved,
                    },
                    _ => moved,
                }
            }
            InputEvent::Button { button, down } => {
                remote
                    .notify_pointer_button(
                        session,
                        button_code(*button),
                        key_state(*down),
                        Default::default(),
                    )
                    .await
            }
            // Up is positive in the core (C5); the portal's steps go down.
            InputEvent::Wheel { dx, dy } => {
                let mut result = Ok(());
                if *dy != 0 {
                    let steps = -(i32::from(*dy).abs() / 120).max(1) * i32::from(dy.signum());
                    result = remote
                        .notify_pointer_axis_discrete(
                            session,
                            Axis::Vertical,
                            steps,
                            Default::default(),
                        )
                        .await;
                }
                if *dx != 0 && result.is_ok() {
                    let steps = (i32::from(*dx).abs() / 120).max(1) * i32::from(dx.signum());
                    result = remote
                        .notify_pointer_axis_discrete(
                            session,
                            Axis::Horizontal,
                            steps,
                            Default::default(),
                        )
                        .await;
                }
                result
            }
            InputEvent::Key { hid_usage, down } => match evdev_code(*hid_usage) {
                Some(code) if sas.allows(*hid_usage, *down) => {
                    remote
                        .notify_keyboard_keycode(
                            session,
                            i32::from(code),
                            key_state(*down),
                            Default::default(),
                        )
                        .await
                }
                _ => Ok(()),
            },
            InputEvent::Text { text } => {
                let mut result = Ok(());
                for character in text.chars() {
                    let sym = keysym_of(character) as i32;
                    for down in [true, false] {
                        if result.is_ok() {
                            result = remote
                                .notify_keyboard_keysym(
                                    session,
                                    sym,
                                    key_state(down),
                                    Default::default(),
                                )
                                .await;
                        }
                    }
                }
                result
            }
            InputEvent::Touch { .. } | InputEvent::Action { .. } => Ok(()),
        };
        if sent.is_err() {
            granted.invalidate();
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_task_wayland_stream_points_and_no_input_before_consent() {
        assert_eq!(stream_point((1920.0, 1080.0), 0, 0), (0.0, 0.0));
        assert_eq!(
            stream_point((1920.0, 1080.0), 65_535, 65_535),
            (1919.0, 1079.0)
        );
        // Before the dialog's consent nothing reaches the session.
        *INPUT.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let text = [InputEvent::Text { text: "x".into() }];
        assert_eq!(inject(&text), Err(ControlBlock::ControlUnavailable));
        assert_eq!(button_code(PointerButton::Left), 0x110);
    }
}
