//! The retained output of a duplication, its device and one duplication of
//! it (moved out of `capture_dxgi.rs` for plan 2026-11-17 P9, which keeps
//! that file under 500 lines). P9: every way a duplication fails names its
//! step (`DxgiFailure`), and in a user session the thread that duplicates
//! is put on the input desktop first (`dxgi_desktop.rs`).
use std::fmt;
use std::io;
use std::time::Instant;

use super::output::{find_output, rotation};
use crate::screen::core::capture_admission::CaptureSlice;
use crate::screen::core::capture_fallback::FallbackStep;
use crate::screen::core::pixel_convert::{BgraRotation, FrameSize};
use crate::screen::os::capture_slots::{CaptureBudget, CaptureGeometry, CaptureKind};
use crate::screen::os::d3d_device::{self, DeviceSet};
use windows::core::HRESULT;
use windows::Win32::Foundation::E_ACCESSDENIED;
use windows::Win32::Graphics::Dxgi::{
    IDXGIOutput1, IDXGIOutputDuplication, DXGI_ERROR_NOT_CURRENTLY_AVAILABLE,
    DXGI_ERROR_SESSION_DISCONNECTED,
};

pub(in crate::screen::os) struct PreparedDxgi {
    pub(super) output: IDXGIOutput1,
    pub(super) devices: DeviceSet,
    pub(super) device: String,
    monitor: usize,
}
impl PreparedDxgi {
    pub(in crate::screen::os) fn prepare(device: &str) -> io::Result<Self> {
        let (adapter, output) = find_output(device)?;
        // Device creation has no requested capture pixels. The actual feature
        // level, queried here, determines the metadata-only texture limits.
        let devices = d3d_device::create_on(Some(&adapter)).map_err(io::Error::other)?;
        let monitor = unsafe { output.GetDesc() }
            .map_err(io::Error::other)?
            .Monitor
            .0 as usize;
        let prepared = Self {
            output,
            devices,
            device: device.into(),
            monitor,
        };
        prepared.geometry()?;
        Ok(prepared)
    }
    pub(in crate::screen::os) fn monitor(&self) -> usize {
        self.monitor
    }
    pub(in crate::screen::os) fn geometry(&self) -> io::Result<CaptureGeometry> {
        // SAFETY: immutable query on the retained output, never a name rebind.
        let desc = unsafe { self.output.GetDesc() }.map_err(io::Error::other)?;
        let end = desc
            .DeviceName
            .iter()
            .position(|u| *u == 0)
            .unwrap_or(desc.DeviceName.len());
        if !desc.AttachedToDesktop.as_bool()
            || desc.Monitor.0 as usize != self.monitor
            || String::from_utf16_lossy(&desc.DeviceName[..end]) != self.device
        {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "original DXGI output changed",
            ));
        }
        let r = desc.DesktopCoordinates;
        let width = r
            .right
            .checked_sub(r.left)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or_else(|| io::Error::other("DXGI output width unavailable"))?;
        let height = r
            .bottom
            .checked_sub(r.top)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or_else(|| io::Error::other("DXGI output height unavailable"))?;
        let rotation = rotation(desc.Rotation)?;
        let native = match rotation {
            BgraRotation::Clockwise90 | BgraRotation::Clockwise270 => FrameSize::new(height, width),
            _ => FrameSize::new(width, height),
        };
        let limits = self.devices.capture_limits(native)?;
        Ok(CaptureGeometry {
            native,
            delivered: self.devices.delivered_size(native.width, native.height)?,
            limits,
            kind: CaptureKind::Dxgi(rotation),
        })
    }
}

/// A failed step of a duplication and its error.
pub(in crate::screen::os) struct DxgiFailure {
    pub step: FallbackStep,
    pub error: io::Error,
}

impl DxgiFailure {
    pub(in crate::screen::os) fn at(step: FallbackStep) -> impl FnOnce(io::Error) -> Self {
        move |error| Self { step, error }
    }
}

impl fmt::Display for DxgiFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.step.key(), self.error)
    }
}

impl From<DxgiFailure> for io::Error {
    fn from(failure: DxgiFailure) -> Self {
        failure.error
    }
}

pub(super) struct Live {
    pub(super) duplication: IDXGIOutputDuplication,
    _native: CaptureSlice,
}

/// One duplication of `prepared` under `budget`; with `follow_desktop` the
/// thread goes to the input desktop first.
pub(super) fn duplicate(
    prepared: &PreparedDxgi,
    budget: &CaptureBudget,
    follow_desktop: bool,
) -> Result<Live, DxgiFailure> {
    let checked = DxgiFailure::at(FallbackStep::OutputCheck);
    let metadata = prepared.geometry().map_err(checked)?;
    if metadata.native != budget.geometry().native || metadata.kind != budget.geometry().kind {
        return Err(DxgiFailure {
            step: FallbackStep::OutputCheck,
            error: io::Error::other("DXGI mode changed before activation"),
        });
    }
    let slice = budget
        .claim_native()
        .map_err(DxgiFailure::at(FallbackStep::CaptureMemory))?;
    if follow_desktop {
        super::desktop::attach();
    }
    // SAFETY: the prepared device is on this retained output's adapter.
    let duplication = unsafe { prepared.output.DuplicateOutput(&prepared.devices.device) }
        .map_err(|error| DxgiFailure {
            step: FallbackStep::DuplicateOutput,
            error: io::Error::other(error),
        })?;
    let desc = unsafe { duplication.GetDesc() };
    let surface = rotation(desc.Rotation).map_err(DxgiFailure::at(FallbackStep::OutputCheck))?;
    let wanted = match metadata.kind {
        CaptureKind::Dxgi(r) => r,
        _ => BgraRotation::Identity,
    };
    if FrameSize::new(desc.ModeDesc.Width, desc.ModeDesc.Height) != metadata.native
        || surface != wanted
    {
        return Err(DxgiFailure {
            step: FallbackStep::OutputCheck,
            error: io::Error::other("DXGI surface metadata changed during activation"),
        });
    }
    Ok(Live {
        duplication,
        _native: slice,
    })
}

/// Since when, in which step and with which result rebuilding fails.
#[derive(Clone, Copy)]
pub(super) struct Failing {
    pub(super) since: Instant,
    pub(super) step: FallbackStep,
    pub(super) kind: io::ErrorKind,
    pub(super) code: Option<HRESULT>,
}

/// Whether a duplication that failed this way may work on the output found
/// again with a new device: a changed output description, or a refusal that
/// is neither the secure desktop, an inactive session nor the limit of
/// concurrent duplications (those wait as before).
pub(super) fn stale(failure: &DxgiFailure) -> bool {
    match failure.step {
        FallbackStep::OutputCheck => true,
        FallbackStep::DuplicateOutput => {
            crate::screen::os::capture_source::result_code(&failure.error).is_some_and(|code| {
                code != E_ACCESSDENIED
                    && code != DXGI_ERROR_SESSION_DISCONNECTED
                    && code != DXGI_ERROR_NOT_CURRENTLY_AVAILABLE
            })
        }
        _ => false,
    }
}
