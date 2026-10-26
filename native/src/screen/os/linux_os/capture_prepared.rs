//! Linux source preparations. CPU pixels require an exact-source image grant;
//! PipeWire preparation may hold independently admitted native imports.
use super::super::{
    portal_session::{PortalOptions, PortalSource},
    x11_capture::X11Source,
};
use crate::screen::core::capture_admission::{CaptureGrant, SessionAdmission, SourceKey};
use crate::screen::core::host_backend::{
    HostOutputPlan, PreparedSource, PreparedSourceInfo, ScreenSource, SourceRefusal,
};
use std::io;
use std::sync::{Arc, Mutex};

pub(crate) struct X11Prepared {
    source: X11Source,
    key: SourceKey,
    output: Mutex<Option<HostOutputPlan>>,
}
impl X11Prepared {
    pub(crate) fn new(
        source: &crate::screen::core::window_list::SourceId,
        key: SourceKey,
        admission: Arc<SessionAdmission>,
        family: bool,
    ) -> Result<Self, SourceRefusal> {
        Ok(Self {
            source: X11Source::prepare(source, key.clone(), admission, family)?,
            key,
            output: Mutex::new(None),
        })
    }
}
impl PreparedSource for X11Prepared {
    fn source_key(&self) -> &SourceKey {
        &self.key
    }
    fn description(&self) -> PreparedSourceInfo {
        self.source.prepared_info()
    }
    fn capture_plan(
        &self,
        output: &HostOutputPlan,
    ) -> io::Result<Option<crate::screen::core::capture_plan::CapturePlan>> {
        let plan = self.source.plan(Some(output))?;
        *self
            .output
            .lock()
            .map_err(|_| io::Error::other("x11 prepared output poisoned"))? = Some(output.clone());
        Ok(Some(plan))
    }
    fn activate(
        self: Box<Self>,
        grant: CaptureGrant,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        let Self {
            source,
            key: _,
            output,
        } = *self;
        let output = output
            .into_inner()
            .map_err(|_| SourceRefusal::new("x11 prepared output poisoned"))?
            .ok_or_else(|| SourceRefusal::new("x11 output plan missing"))?;
        source
            .activate(grant, Some(output))
            .map(|source| Box::new(source) as Box<dyn ScreenSource>)
    }
}
pub(super) struct PortalPrepared {
    source: PortalSource,
}
impl PortalPrepared {
    pub(super) fn new(
        options: PortalOptions,
        key: SourceKey,
        admission: Arc<SessionAdmission>,
    ) -> io::Result<Self> {
        Ok(Self {
            source: PortalSource::prepare(options, key, admission)?,
        })
    }
}
impl PreparedSource for PortalPrepared {
    fn source_key(&self) -> &SourceKey {
        self.source.memory().source()
    }
    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        self.source.set_capture_active(active)
    }
    fn input_sink(&self) -> Option<Arc<dyn crate::screen::core::host_backend::ScreenInputSink>> {
        self.source.input_sink()
    }
    fn description(&self) -> PreparedSourceInfo {
        use crate::screen::core::{frame_budget::FrameLimits, session_state::SessionState};
        PreparedSourceInfo {
            native_size: self.source.memory().size(),
            capture_limits: FrameLimits::RESOURCE,
            supports_regions: true,
            state: SessionState::WaitingForPortal,
            generation: self.source.memory().generation(),
        }
    }
    fn capture_plan(
        &self,
        output: &HostOutputPlan,
    ) -> io::Result<Option<crate::screen::core::capture_plan::CapturePlan>> {
        self.source.check_preparation()?;
        self.source.memory().capture_plan(output)
    }
    fn activate(
        self: Box<Self>,
        grant: CaptureGrant,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        self.source
            .check_preparation()
            .and_then(|()| self.source.memory().activate(grant))
            .map_err(|error| SourceRefusal::new(&error.to_string()))?;
        Ok(Box::new(self.source))
    }
}

#[cfg(test)]
pub(crate) fn test_admission(session: &str) -> Arc<SessionAdmission> {
    use crate::screen::core::frame_memory::{CapacityCeiling, MemoryFacts, MemoryPool};
    let facts = Some(MemoryFacts {
        available_physical: 8 << 30,
        available_commit: 8 << 30,
        available_address_space: 8 << 30,
    });
    let admission = Arc::new(MemoryPool::default())
        .admit_session(
            facts,
            CapacityCeiling::desktop(facts),
            crate::screen::core::frame_budget::MAX_WORKING_BYTES,
            session,
        )
        .unwrap();
    admission.bind_contract(None).unwrap();
    admission
}

/// Sends one batch; a failing call drops the rest of the batch.
pub(in crate::screen::os) async fn apply_portal_input(
    remote: &ashpd::desktop::remote_desktop::RemoteDesktop,
    session: &ashpd::desktop::Session<ashpd::desktop::remote_desktop::RemoteDesktop>,
    granted: &crate::screen::os::pipewire_stream::identity::StreamIdentity,
    session_path: &str,
    batch: &[crate::screen::core::input_model::InputEvent],
    sas: &mut crate::screen::os::input_keys::SasGuard,
) {
    use crate::screen::core::input_model::{InputEvent, TouchPhase};
    use crate::screen::os::input_keys::evdev_code;
    use crate::screen::os::portal_input::{button_code, key_state, stream_point};
    use crate::screen::os::x11_keymap::keysym_of;
    use ashpd::desktop::remote_desktop::Axis;
    for event in batch {
        if !granted.belongs_to(session_path) {
            return;
        }
        let sent = match event {
            InputEvent::PointerAbs { x, y } | InputEvent::Touch { id: 0, x, y, .. } => {
                // Missing logical size is not permission to use pixel/global-display coordinates.
                let Some((width, height)) = granted.logical_size else {
                    return;
                };
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

/// Release-only portal notification. The caller has matched the exact
/// InputLease identity to its own session; capture expiry is not a new target.
pub(in crate::screen::os) async fn apply_portal_release(
    remote: &ashpd::desktop::remote_desktop::RemoteDesktop,
    session: &ashpd::desktop::Session<ashpd::desktop::remote_desktop::RemoteDesktop>,
    events: &[crate::screen::core::input_model::InputEvent],
    sas: &mut crate::screen::os::input_keys::SasGuard,
) {
    use crate::screen::core::input_model::{InputEvent, PointerButton, TouchPhase};
    use crate::screen::os::portal_input::{button_code, key_state};
    for event in events {
        let result = match event {
            InputEvent::Key {
                hid_usage,
                down: false,
            } => {
                let _ = sas.allows(*hid_usage, false);
                let Some(code) = crate::screen::os::input_keys::evdev_code(*hid_usage) else {
                    continue;
                };
                remote
                    .notify_keyboard_keycode(
                        session,
                        i32::from(code),
                        key_state(false),
                        Default::default(),
                    )
                    .await
            }
            InputEvent::Button {
                button,
                down: false,
            } => {
                remote
                    .notify_pointer_button(
                        session,
                        button_code(*button),
                        key_state(false),
                        Default::default(),
                    )
                    .await
            }
            InputEvent::Touch {
                id: 0,
                phase: TouchPhase::End | TouchPhase::Cancel,
                ..
            } => {
                remote
                    .notify_pointer_button(
                        session,
                        button_code(PointerButton::Left),
                        key_state(false),
                        Default::default(),
                    )
                    .await
            }
            _ => return,
        };
        if result.is_err() {
            return;
        }
    }
}

pub(in crate::screen::os) fn portal_input_sink(
    lease: Arc<crate::screen::os::portal_input::InputLease>,
    memory: &Arc<crate::screen::os::pipewire_stream::memory::CaptureMemory>,
) -> Arc<dyn crate::screen::core::host_backend::ScreenInputSink> {
    Arc::new(PortalInputSink {
        lease,
        memory: Arc::downgrade(memory),
    })
}
struct PortalInputSink {
    lease: Arc<crate::screen::os::portal_input::InputLease>,
    memory: std::sync::Weak<crate::screen::os::pipewire_stream::memory::CaptureMemory>,
}
impl crate::screen::core::host_backend::ScreenInputSink for PortalInputSink {
    fn inject(
        &self,
        events: &[crate::screen::core::input_model::InputEvent],
        reference_generation: u64,
        _presented: bool,
    ) -> Result<(), crate::screen::core::wire_control::ControlBlock> {
        let memory = self
            .memory
            .upgrade()
            .ok_or(crate::screen::core::wire_control::ControlBlock::ControlUnavailable)?;
        if reference_generation != memory.generation().value() {
            return Err(crate::screen::core::wire_control::ControlBlock::ControlUnavailable);
        }
        crate::screen::os::portal_input::send_input(&self.lease, events)
    }
    fn release(
        &self,
        events: &[crate::screen::core::input_model::InputEvent],
    ) -> Result<(), crate::screen::core::wire_control::ControlBlock> {
        if !crate::screen::os::portal_input::release_only(events) {
            return Err(crate::screen::core::wire_control::ControlBlock::ControlUnavailable);
        }
        if events.is_empty() {
            return Ok(());
        }
        // This lease binds its one identity once, after portal consent. A
        // terminal capture identity still permits release to that same session
        // while its portal receiver lives; no frame grant or global INPUT read.
        let identity = self
            .lease
            .binding
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or(crate::screen::core::wire_control::ControlBlock::ControlUnavailable)?;
        self.lease
            .sender
            .send(crate::screen::os::portal_input::Batch::Release {
                identity,
                events: events.to_vec(),
            })
            .map_err(|_| crate::screen::core::wire_control::ControlBlock::ControlUnavailable)
    }
}

pub(in crate::screen::os) async fn dispatch_portal_batch(
    remote: &ashpd::desktop::remote_desktop::RemoteDesktop,
    session: &ashpd::desktop::Session<ashpd::desktop::remote_desktop::RemoteDesktop>,
    identity: &Arc<crate::screen::os::pipewire_stream::identity::StreamIdentity>,
    path: &str,
    batch: crate::screen::os::portal_input::Batch,
    sas: &mut crate::screen::os::input_keys::SasGuard,
) {
    use crate::screen::os::portal_input::Batch;
    match batch {
        Batch::Input(events) => {
            apply_portal_input(remote, session, identity, path, &events, sas).await
        }
        Batch::Release {
            identity: bound,
            events,
        } if Arc::ptr_eq(&bound, identity) && bound.session == path => {
            apply_portal_release(remote, session, &events, sas).await;
        }
        Batch::Release { .. } => {}
    }
}
