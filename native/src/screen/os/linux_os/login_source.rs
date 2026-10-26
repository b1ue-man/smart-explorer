//! The broker has already bound this process to a verified X11 greeter account.
use crate::screen::core::capture_admission::{CaptureGrant, SessionAdmission, SourceKey};
use crate::screen::core::host_backend::{
    HostOutputPlan, PreparedSource, PreparedSourceInfo, ScreenHostBackend, ScreenSource,
    SourceRefusal,
};
use crate::screen::core::input_model::InputEvent;
use crate::screen::core::window_list::SourceId;
use std::io;
use std::sync::{Arc, Mutex, Weak};
#[path = "login_geometry.rs"]
mod geometry;
static LOGIN: Mutex<Option<Weak<Mutex<geometry::Lease>>>> = Mutex::new(None);

struct LoginSource {
    source: Box<dyn ScreenSource>,
    lease: Arc<Mutex<geometry::Lease>>,
    reference: u64,
}
fn expired() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "login capture geometry changed; start a new login connection",
    )
}
impl ScreenSource for LoginSource {
    fn capture_grant(&self) -> Option<CaptureGrant> {
        self.source.capture_grant()
    }
    fn size(&self) -> crate::screen::core::pixel_convert::FrameSize {
        self.source.size()
    }
    fn frame_limits(&self) -> crate::screen::core::frame_budget::FrameLimits {
        self.source.frame_limits()
    }
    fn reference_generation(&self) -> u64 {
        self.source.reference_generation()
    }
    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        self.source.set_capture_active(active)
    }
    fn capture(
        &mut self,
        frame: u64,
        keyframe: bool,
    ) -> io::Result<crate::screen::core::host_backend::SourceFrame> {
        use crate::screen::core::host_backend::SourceFrame;
        let mut lease = self.lease.lock().map_err(|_| expired())?;
        if !lease.current() {
            return Err(expired());
        }
        let captured = self.source.capture(frame, keyframe)?;
        if self.reference != self.source.reference_generation() || !lease.current() {
            lease.invalidate();
            return Err(expired());
        }
        if let SourceFrame::Frame(image) = &captured {
            let area = lease.area();
            if image.size().width != area.width as u32 || image.size().height != area.height as u32
            {
                lease.invalidate();
                return Err(expired());
            }
            lease.captured = true;
        }
        Ok(captured)
    }
}
impl Drop for LoginSource {
    fn drop(&mut self) {
        if let Ok(mut lease) = self.lease.lock() {
            lease.invalidate();
        }
    }
}

pub(crate) fn login_geometry_current() -> io::Result<bool> {
    let lease = LOGIN
        .lock()
        .map_err(|_| expired())?
        .as_ref()
        .and_then(Weak::upgrade);
    match lease {
        Some(lease) => Ok(lease.lock().map_err(|_| expired())?.current()),
        None => Ok(false),
    }
}

struct PreparedLogin {
    source: Box<dyn PreparedSource>,
    lease: Arc<Mutex<geometry::Lease>>,
}
impl PreparedSource for PreparedLogin {
    fn source_key(&self) -> &SourceKey {
        self.source.source_key()
    }
    fn description(&self) -> PreparedSourceInfo {
        self.source.description()
    }
    fn capture_plan(
        &self,
        output: &HostOutputPlan,
    ) -> io::Result<Option<crate::screen::core::capture_plan::CapturePlan>> {
        if !self.lease.lock().map_err(|_| expired())?.current() {
            return Err(expired());
        }
        self.source.capture_plan(output)
    }
    fn activate(
        self: Box<Self>,
        grant: CaptureGrant,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        let Self { source, lease } = *self;
        if !lease
            .lock()
            .map_err(|_| SourceRefusal::new("login geometry lease poisoned"))?
            .current()
        {
            return Err(SourceRefusal::new("login capture geometry changed"));
        }
        let source = source.activate(grant)?;
        let reference = source.reference_generation();
        *LOGIN
            .lock()
            .map_err(|_| SourceRefusal::new("login geometry registry poisoned"))? =
            Some(Arc::downgrade(&lease));
        Ok(Box::new(LoginSource {
            source,
            lease,
            reference,
        }))
    }
}

pub(crate) fn prepare_login_source(
    admission: Arc<SessionAdmission>,
) -> io::Result<Box<dyn PreparedSource>> {
    if super::session_detect::detect_process() != super::session_detect::SessionKind::X11 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "login helper requires a verified X11 greeter",
        ));
    }
    let backend = super::host_backend::LinuxBackend::new();
    let monitor = backend
        .monitors()
        .into_iter()
        .find(|monitor| monitor.primary)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "X11 greeter monitor unavailable")
        })?;
    let id = SourceId::Monitor(monitor.id);
    let key = SourceKey::new(admission.session(), &format!("login:{id:?}"))?;
    let source = super::host_backend::prepared::X11Prepared::new(&id, key, admission, false)
        .map_err(|error| io::Error::other(error.reason))?;
    let lease = Arc::new(Mutex::new(geometry::Lease::new(id)?));
    let area = lease.lock().map_err(|_| expired())?.area();
    if source.description().native_size
        != Some(crate::screen::core::pixel_convert::FrameSize::new(
            area.width as u32,
            area.height as u32,
        ))
    {
        return Err(expired());
    }
    Ok(Box::new(PreparedLogin {
        source: Box::new(source),
        lease,
    }))
}
/// Old entrypoint refuses before pixels. The helper must use prepare/activate.
pub(crate) fn open_login_source() -> io::Result<Box<dyn ScreenSource>> {
    Err(io::Error::other("login_capture_grant_required"))
}

pub(crate) fn inject_login_input(events: &[InputEvent]) -> io::Result<()> {
    use crate::screen::core::input_model::TouchPhase;
    if events.iter().all(|event| {
        matches!(
            event,
            InputEvent::Key { down: false, .. }
                | InputEvent::Button { down: false, .. }
                | InputEvent::Touch {
                    phase: TouchPhase::End | TouchPhase::Cancel,
                    ..
                }
        )
    }) {
        return super::x11_input::release(events)
            .map_err(|reason| io::Error::other(format!("X11 greeter release: {reason:?}")));
    }
    let lease = LOGIN
        .lock()
        .map_err(|_| expired())?
        .as_ref()
        .and_then(Weak::upgrade)
        .ok_or_else(expired)?;
    let mut lease = lease.lock().map_err(|_| expired())?;
    if !lease.current() || !lease.captured {
        return Err(expired());
    }
    super::x11_input::inject_bound(&lease.source, lease.area(), events).map_err(|reason| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("X11 greeter input: {reason:?}"),
        )
    })
}
