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
use crate::screen::core::capture_plan::CapturePlan;
use crate::screen::core::capture_source_contract::{
    HostOutputPlan, PreparedSource, PreparedSourceInfo,
};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::host_backend::{ScreenSource, SourceRefusal};
use crate::screen::core::session_state::SessionState;
use crate::screen::core::window_list::SourceId;
use std::io;
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow};

#[path = "capture_monitor.rs"]
mod monitor;
#[path = "capture_recovery.rs"]
pub(super) mod recovery;
pub(super) use monitor::{MonitorGrabber, OpenFallback};

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
        let first = monitor_first(&monitor, admission.clone(), key.clone(), &selected.1);
        let grabber: Box<dyn capture_source::Grabber> = match capture {
            PreparedCapture::Dxgi(prepared) => match DxgiCapture::open_stepped(prepared, budget) {
                Ok(capture) => Box::new(MonitorGrabber {
                    current: Box::new(capture.with_pointer().following_input_desktop()),
                    fallback,
                    first,
                    retry: None,
                    back: None,
                }),
                Err(failure) => {
                    // B-12: a duplication refused at activation (for example
                    // `DXGI_ERROR_UNSUPPORTED`) gives way to Graphics Capture
                    // with its own plan; the secure desktop is refused above.
                    // P9: the fallback is temporary (`capture_monitor.rs`).
                    let cause = capture_source::fallback_cause(
                        capture_source::result_code(&failure.error),
                        false,
                        capture_source::FALLBACK_AFTER,
                    );
                    match (cause, fallback) {
                        (Some(cause), Some(mut open)) => {
                            let detail = capture_source::detail_of(failure.step, &failure.error);
                            capture_source::update_report(|report| {
                                report.fallback = Some(cause);
                                report.detail = Some(detail);
                            });
                            let current = open(1).map_err(refused)?;
                            let retry = first.is_some().then(|| {
                                crate::screen::core::capture_fallback::RetryPlan::new(
                                    std::time::Instant::now(),
                                )
                            });
                            Box::new(MonitorGrabber {
                                current,
                                fallback: Some(open),
                                first,
                                retry,
                                back: None,
                            }) as Box<dyn Grabber>
                        }
                        _ => return Err(refused(failure)),
                    }
                }
            },
            PreparedCapture::Wgc(prepared) => match WgcCapture::open(prepared, budget) {
                Ok(capture) => Box::new(capture),
                Err(error) => {
                    // WGC can fail at StartCapture. A monitor retains its DXGI
                    // fallback, but obtains that backend's exact new plan first.
                    eprintln!("screen capture: Graphics Capture did not start: {error}");
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
                            .with_pointer()
                            .following_input_desktop(),
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
        let budget = CaptureBudget::activate(
            admission.clone(),
            key.clone(),
            geometry,
            output.clone(),
            grant,
        )?;
        Ok(Box::new(WgcCapture::open(prepared, budget)?) as Box<dyn Grabber>)
    }))
}

/// P9: Desktop Duplication of a monitor again, after a fallback, with its own
/// exact plan and grant and the generation after `previous`.
fn monitor_first(
    monitor: &Option<(String, usize)>,
    admission: Arc<SessionAdmission>,
    key: SourceKey,
    output: &HostOutputPlan,
) -> Option<OpenFallback> {
    let (device, _) = monitor.clone()?;
    let output = output.clone();
    Some(Box::new(move |previous: u64| {
        let generation = previous
            .checked_add(1)
            .ok_or_else(|| io::Error::other("capture generation exhausted"))?;
        let prepared = PreparedDxgi::prepare(&device)?;
        let geometry = prepared.geometry()?;
        let plan = geometry.plan(key.clone(), CaptureGeneration::new(generation), &output)?;
        let grant = admission.authority().approve(plan)?;
        let budget = CaptureBudget::activate(
            admission.clone(),
            key.clone(),
            geometry,
            output.clone(),
            grant,
        )?;
        let capture = DxgiCapture::open_stepped(prepared, budget)?;
        Ok(Box::new(capture.with_pointer().following_input_desktop()) as Box<dyn Grabber>)
    }))
}
