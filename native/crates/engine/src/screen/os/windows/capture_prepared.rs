//! Metadata-only preparation of one retained Windows source. The recorded
//! output plan is bound before activate can create a framepool or duplication.
use super::capture_dxgi::{DxgiCapture, PreparedDxgi};
use super::capture_slots::{CaptureBudget, CaptureGeometry};
use super::capture_source::{self, CapturePath, Grabber, WindowsSource};
use super::capture_wgc::{self, PreparedWgc, Target, WgcCapture};
use super::{monitors, secure_desktop};
use crate::screen::core::capture_admission::{
    CaptureGeneration, CaptureGrant, SessionAdmission, SourceKey,
};
use crate::screen::core::capture_picture::SourcePicture;
use crate::screen::core::capture_plan::CapturePlan;
use crate::screen::core::capture_source_contract::{
    HostOutputPlan, PreparedSource, PreparedSourceInfo,
};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::host_backend::SourceFrame;
use crate::screen::core::host_backend::{ScreenSource, SourceRefusal};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::window_list::SourceId;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow};

#[path = "capture_recovery.rs"]
pub(super) mod recovery;

enum PreparedCapture {
    Wgc(PreparedWgc),
    Dxgi(PreparedDxgi),
}
impl PreparedCapture {
    fn geometry(&self) -> io::Result<Option<CaptureGeometry>> {
        match self {
            Self::Wgc(prepared) => prepared.geometry(),
            Self::Dxgi(prepared) => prepared.geometry().map(Some),
        }
    }
}
pub(super) struct PreparedWindows {
    capture: PreparedCapture,
    source: SourceId,
    monitor: Option<(String, usize)>,
    key: SourceKey,
    admission: Arc<SessionAdmission>,
    selected: Mutex<Option<(CaptureGeometry, HostOutputPlan)>>,
}
fn refused(error: impl std::fmt::Display) -> SourceRefusal {
    SourceRefusal::new(&error.to_string())
}
impl PreparedWindows {
    pub(super) fn prepare(
        session: &str,
        source: &SourceId,
        admission: Arc<SessionAdmission>,
    ) -> Result<Box<dyn PreparedSource>, SourceRefusal> {
        if session != admission.session() {
            return Err(refused(
                "Windows capture admission belongs to another session",
            ));
        }
        monitors::ensure_dpi_aware();
        super::mf_common::ensure_com();
        if !secure_desktop::session_active() {
            return Err(refused(capture_source::SESSION_INACTIVE));
        }
        let key = SourceKey::new(session, &source.to_string()).map_err(refused)?;
        let (target, monitor) = match source {
            SourceId::Monitor(id) => {
                let monitor =
                    monitors::find(id).ok_or_else(|| refused(capture_source::MONITOR_NOT_FOUND))?;
                (
                    Target::Monitor(id.clone()),
                    Some((monitor.device, monitor.handle.0 as usize)),
                )
            }
            SourceId::Window(id) => {
                let handle: usize = id
                    .parse()
                    .map_err(|_| refused(capture_source::WINDOW_NOT_FOUND))?;
                if !unsafe { IsWindow(HWND(handle as *mut _)) }.as_bool() {
                    return Err(refused(capture_source::WINDOW_NOT_FOUND));
                }
                (Target::Window(handle), None)
            }
            SourceId::Synthetic => return Err(refused(capture_source::CAPTURE_UNAVAILABLE)),
        };
        // Plan 2026-11-14 bildkette B-12: a monitor with Desktop Duplication
        // first, Graphics Capture when the duplication cannot be prepared
        // (another adapter, no output); a window only with Graphics Capture.
        let mut capture = None;
        let mut refusal = None;
        for path in capture_source::capture_rank(monitor.is_some(), capture_wgc::available()) {
            let prepared = match (path, &monitor) {
                (CapturePath::DesktopDuplication, Some((device, handle))) => {
                    match PreparedDxgi::prepare(device) {
                        Ok(prepared) if prepared.monitor() == *handle => {
                            Ok(PreparedCapture::Dxgi(prepared))
                        }
                        Ok(_) => Err(refused("original capture monitor changed")),
                        Err(error) => Err(refused(error)),
                    }
                }
                (CapturePath::DesktopDuplication, None) => continue,
                (CapturePath::GraphicsCapture, _) => PreparedWgc::prepare(&target)
                    .map(PreparedCapture::Wgc)
                    .map_err(|_| refused(capture_source::WINDOW_CAPTURE_UNSUPPORTED)),
            };
            match prepared {
                Ok(prepared) => {
                    capture = Some(prepared);
                    break;
                }
                // The first refusal of a monitor is the duplication's, as before.
                Err(error) => {
                    if refusal.is_none() {
                        refusal = Some(error);
                    }
                }
            }
        }
        let capture = capture.ok_or_else(|| {
            refusal.unwrap_or_else(|| refused(capture_source::WINDOW_CAPTURE_UNSUPPORTED))
        })?;
        Ok(Box::new(Self {
            capture,
            source: source.clone(),
            monitor,
            key,
            admission,
            selected: Mutex::new(None),
        }))
    }
    fn recheck(&self) -> io::Result<()> {
        if !secure_desktop::session_active() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                capture_source::SESSION_INACTIVE,
            ));
        }
        if secure_desktop::on_secure_desktop() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "secure_desktop_paused",
            ));
        }
        if let SourceId::Window(id) = &self.source {
            let handle: usize = id.parse().map_err(io::Error::other)?;
            if !unsafe { IsWindow(HWND(handle as *mut _)) }.as_bool() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    capture_source::WINDOW_NOT_FOUND,
                ));
            }
        }
        if let (SourceId::Monitor(id), Some((device, handle))) = (&self.source, &self.monitor) {
            if !monitors::find(id)
                .is_some_and(|m| m.device == *device && m.handle.0 as usize == *handle)
            {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "original capture monitor changed",
                ));
            }
        }
        Ok(())
    }
}
impl PreparedSource for PreparedWindows {
    fn source_key(&self) -> &SourceKey {
        &self.key
    }
    fn description(&self) -> PreparedSourceInfo {
        let geometry = self.capture.geometry().ok().flatten();
        let state = if !secure_desktop::session_active() {
            SessionState::SessionInactive
        } else if secure_desktop::on_secure_desktop() {
            SessionState::SecureDesktop
        } else if matches!(&self.source,SourceId::Window(id) if id.parse::<usize>().ok()
                .is_some_and(|handle|unsafe {IsIconic(HWND(handle as *mut _))}.as_bool()))
        {
            SessionState::WindowMinimized
        } else {
            SessionState::Starting
        };
        PreparedSourceInfo {
            native_size: geometry.map(|g| g.native_displayed()),
            capture_limits: geometry.map_or(FrameLimits::RESOURCE, |g| g.limits),
            supports_regions: true,
            state,
            generation: CaptureGeneration::new(1),
        }
    }
    fn capture_plan(&self, output: &HostOutputPlan) -> io::Result<Option<CapturePlan>> {
        self.recheck()?;
        let Some(geometry) = self.capture.geometry()? else {
            return Ok(None);
        };
        let plan = geometry.plan(self.key.clone(), CaptureGeneration::new(1), output)?;
        let mut selected = self
            .selected
            .lock()
            .map_err(|_| io::Error::other("Windows preparation poisoned"))?;
        if selected.as_ref().is_some_and(|(_, old)| {
            old.contract() != output.contract()
                || old.size() != output.size()
                || old.codec() != output.codec()
        }) {
            return Err(io::Error::other(
                "Windows prepared output contract already selected",
            ));
        }
        *selected = Some((geometry, output.clone()));
        Ok(Some(plan))
    }
    fn activate(
        self: Box<Self>,
        grant: CaptureGrant,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        self.recheck().map_err(refused)?;
        self.description()
            .check_grant(&self.key, &grant)
            .map_err(refused)?;
        let selected = self
            .selected
            .lock()
            .map_err(|_| refused("Windows preparation poisoned"))?
            .take()
            .ok_or_else(|| refused("Windows output plan not prepared"))?;
        let current = self
            .capture
            .geometry()
            .map_err(refused)?
            .ok_or_else(|| refused("Windows capture metadata pending"))?;
        if current.native != selected.0.native || current.kind != selected.0.kind {
            return Err(refused("Windows geometry changed before activation"));
        }
        let Self {
            capture,
            source,
            monitor,
            key,
            admission,
            ..
        } = *self;
        let budget = CaptureBudget::activate(
            admission.clone(),
            key.clone(),
            current,
            selected.1.clone(),
            grant,
        )
        .map_err(refused)?;
        let fallback = monitor_fallback(&source, admission.clone(), key.clone(), &selected.1);
        let grabber: Box<dyn capture_source::Grabber> = match capture {
            PreparedCapture::Dxgi(prepared) => match DxgiCapture::open(prepared, budget) {
                Ok(capture) => Box::new(MonitorGrabber {
                    current: Box::new(capture.with_pointer()),
                    fallback,
                }),
                Err(error) => {
                    // B-12: a duplication refused at activation (for example
                    // `DXGI_ERROR_UNSUPPORTED`) gives way to Graphics Capture
                    // with its own plan; the secure desktop is refused above.
                    let cause = capture_source::fallback_cause(
                        capture_source::result_code(&error),
                        false,
                        capture_source::FALLBACK_AFTER,
                    );
                    match (cause, fallback) {
                        (Some(cause), Some(open)) => {
                            let grabber = open(1).map_err(refused)?;
                            capture_source::update_report(|report| report.fallback = Some(cause));
                            grabber
                        }
                        _ => return Err(refused(error)),
                    }
                }
            },
            PreparedCapture::Wgc(prepared) => match WgcCapture::open(prepared, budget) {
                Ok(capture) => Box::new(capture),
                Err(error) => {
                    // WGC can fail at StartCapture. A monitor retains its DXGI
                    // fallback, but obtains that backend's exact new plan first.
                    let Some((device, handle)) = monitor else {
                        return Err(refused(error));
                    };
                    let prepared = PreparedDxgi::prepare(&device).map_err(refused)?;
                    if prepared.monitor() != handle {
                        return Err(refused("original fallback monitor changed"));
                    }
                    let geometry = prepared.geometry().map_err(refused)?;
                    let plan = geometry
                        .plan(key.clone(), CaptureGeneration::new(2), &selected.1)
                        .map_err(refused)?;
                    let grant = admission.authority().approve(plan).map_err(refused)?;
                    let budget =
                        CaptureBudget::activate(admission, key, geometry, selected.1, grant)
                            .map_err(refused)?;
                    Box::new(
                        DxgiCapture::open(prepared, budget)
                            .map_err(refused)?
                            .with_pointer(),
                    )
                }
            },
        };
        let window = match source {
            SourceId::Window(id) => id.parse().ok(),
            _ => None,
        };
        Ok(Box::new(WindowsSource::activated(grabber, window)))
    }
}

/// Opens Graphics Capture for a monitor with the generation after `previous`.
pub(super) type OpenFallback = Box<dyn FnOnce(u64) -> io::Result<Box<dyn Grabber>> + Send>;

/// The Graphics Capture fallback of a monitor source (B-12), where Windows
/// has Graphics Capture; it asks for its own exact plan and grant.
fn monitor_fallback(
    source: &SourceId,
    admission: Arc<SessionAdmission>,
    key: SourceKey,
    output: &HostOutputPlan,
) -> Option<OpenFallback> {
    let SourceId::Monitor(id) = source else {
        return None;
    };
    if !capture_wgc::available() {
        return None;
    }
    let target = Target::Monitor(id.clone());
    let output = output.clone();
    Some(Box::new(move |previous: u64| {
        let generation = previous
            .checked_add(1)
            .ok_or_else(|| io::Error::other("capture generation exhausted"))?;
        let prepared = PreparedWgc::prepare(&target)?;
        let geometry = prepared
            .geometry()?
            .ok_or_else(|| io::Error::other("WGC native metadata pending"))?;
        let plan = geometry.plan(key.clone(), CaptureGeneration::new(generation), &output)?;
        let grant = admission.authority().approve(plan)?;
        let budget = CaptureBudget::activate(admission, key, geometry, output, grant)?;
        Ok(Box::new(WgcCapture::open(prepared, budget)?) as Box<dyn Grabber>)
    }))
}

/// A monitor on Desktop Duplication that gives way to Graphics Capture once
/// the duplication asks for it (`Grabber::fallback_cause`): unsupported for
/// longer than `FALLBACK_AFTER`, lost and not rebuilt, or its output left
/// the adapter. The session continues with a new generation, so the host
/// process starts its encoder anew.
pub(super) struct MonitorGrabber {
    pub(super) current: Box<dyn Grabber>,
    pub(super) fallback: Option<OpenFallback>,
}

impl MonitorGrabber {
    /// Switches when the duplication asks for it; true after a switch.
    fn fall_back(&mut self) -> io::Result<bool> {
        let Some(cause) = self.current.fallback_cause() else {
            return Ok(false);
        };
        let Some(open) = self.fallback.take() else {
            return Ok(false);
        };
        let previous = self.current.reference_generation();
        // The duplication and its grant end before Graphics Capture asks for
        // its own grant.
        self.current = Box::new(Retired {
            size: self.current.size(),
        });
        self.current = open(previous)?;
        capture_source::update_report(|report| report.fallback = Some(cause));
        Ok(true)
    }
}

impl Grabber for MonitorGrabber {
    fn size(&self) -> FrameSize {
        self.current.size()
    }
    fn capture_grant(&self) -> Option<CaptureGrant> {
        self.current.capture_grant()
    }
    fn frame_limits(&self) -> FrameLimits {
        self.current.frame_limits()
    }
    fn reference_generation(&self) -> u64 {
        self.current.reference_generation()
    }
    fn source_available(&self) -> bool {
        self.current.source_available()
    }
    fn path(&self) -> Option<CapturePath> {
        self.current.path()
    }
    fn set_frame_interval(&mut self, interval: Duration) -> bool {
        self.current.set_frame_interval(interval)
    }
    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        self.current.set_capture_active(active)
    }
    fn capture(&mut self, keyframe: bool) -> io::Result<SourceFrame> {
        self.capture_picture(keyframe)?.into_source_frame()
    }
    fn capture_waiting(
        &mut self,
        keyframe: bool,
        wait: Duration,
        release: &mut dyn FnMut() -> bool,
    ) -> io::Result<SourceFrame> {
        self.capture_waiting_picture(keyframe, wait, release)?
            .into_source_frame()
    }
    /// The current grabber's picture, on the GPU where it stays there (BK3b).
    fn capture_picture(&mut self, keyframe: bool) -> io::Result<SourcePicture> {
        let result = self.current.capture_picture(keyframe);
        if self.fall_back()? {
            return Ok(SourcePicture::Unchanged);
        }
        result
    }
    fn capture_waiting_picture(
        &mut self,
        keyframe: bool,
        wait: Duration,
        release: &mut dyn FnMut() -> bool,
    ) -> io::Result<SourcePicture> {
        let result = self
            .current
            .capture_waiting_picture(keyframe, wait, release);
        if self.fall_back()? {
            return Ok(SourcePicture::Unchanged);
        }
        result
    }
    /// After the secure desktop the duplication may be refused for a while
    /// with a documented passing result: the source then stays without a
    /// duplication and the next capture rebuilds or pauses, as during a
    /// capture; any other refusal falls back or ends the session.
    fn reopen(&mut self) -> io::Result<()> {
        let Err(error) = self.current.reopen() else {
            return Ok(());
        };
        if self.fall_back()? {
            return Ok(());
        }
        recovery::CaptureRecovery::paused_by_duplication(error).map(|_| ())
    }
}

/// The place of a duplication whose fallback could not open.
struct Retired {
    size: FrameSize,
}

impl Grabber for Retired {
    fn size(&self) -> FrameSize {
        self.size
    }
    fn capture(&mut self, _keyframe: bool) -> io::Result<SourceFrame> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "capture fallback unavailable",
        ))
    }
    fn reopen(&mut self) -> io::Result<()> {
        self.capture(false).map(|_| ())
    }
    fn source_available(&self) -> bool {
        false
    }
}
