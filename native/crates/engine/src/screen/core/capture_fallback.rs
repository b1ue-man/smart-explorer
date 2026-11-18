//! Why a capture left its first path, as text, and when it tries that path
//! again (plan 2026-11-17 P9, „Aufnahme: dxgi failed → Rückfall“).
//!
//! - `FallbackDetail`: the step that failed (`FallbackStep`, the Windows API
//!   call or the check before it) and its result code. It crosses the process
//!   boundary in the bits 24–63 of the packed capture facts
//!   (`CaptureFacts::pack` uses bits 0–21 and ignores the rest on
//!   `unpack`, so an older daemon reads the facts unchanged) and reaches the
//!   viewer's chain facts as `ChainReport::capture_fallback` text.
//! - `RetryPlan`: a fallback is temporary. The first path is tried again
//!   after `RETRY_FIRST`, every failed try doubles the wait up to
//!   `RETRY_LONGEST`. The bound is no rate limit of the picture: a failed
//!   try costs the session one new encoder (the generation changes twice),
//!   so the bound keeps a first path that never comes back from restarting
//!   the encoder more often than once a minute.
use std::time::{Duration, Instant};

use super::frame_timing::{CaptureFallback, CapturePath};

/// The step of a capture that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FallbackStep {
    /// `IDXGIOutput1::DuplicateOutput`.
    DuplicateOutput,
    /// `IDXGIOutputDuplication::AcquireNextFrame`.
    AcquireNextFrame,
    /// `IDXGIOutputDuplication::ReleaseFrame`.
    ReleaseFrame,
    /// The output's description: attached, same monitor, same mode.
    OutputCheck,
    /// The capture memory of the session (`CaptureBudget`).
    CaptureMemory,
    /// Finding the output again and creating its D3D11 device.
    DeviceCreate,
    /// Putting the capture thread on the input desktop.
    DesktopAttach,
    /// The first duplication of a session (activation).
    Activate,
}

impl FallbackStep {
    /// The packing index is the position here plus one: new steps go to the end.
    pub const ALL: [Self; 8] = [
        Self::DuplicateOutput,
        Self::AcquireNextFrame,
        Self::ReleaseFrame,
        Self::OutputCheck,
        Self::CaptureMemory,
        Self::DeviceCreate,
        Self::DesktopAttach,
        Self::Activate,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::DuplicateOutput => "DuplicateOutput",
            Self::AcquireNextFrame => "AcquireNextFrame",
            Self::ReleaseFrame => "ReleaseFrame",
            Self::OutputCheck => "output_check",
            Self::CaptureMemory => "capture_memory",
            Self::DeviceCreate => "device_create",
            Self::DesktopAttach => "desktop_attach",
            Self::Activate => "activate",
        }
    }
}

const STEP_SHIFT: u32 = 24;
const CODE_SHIFT: u32 = 32;

/// The step and result code of the failure that caused a fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FallbackDetail {
    pub step: FallbackStep,
    /// The `HRESULT` as its 32 bits; `None` for an error without one.
    pub code: Option<u32>,
}

impl FallbackDetail {
    /// Bits 24–31 the step (index plus one), 32–63 the result code (0: none;
    /// `S_OK` is never the code of a failure).
    pub fn pack(&self) -> u64 {
        let index = FallbackStep::ALL
            .iter()
            .position(|step| *step == self.step)
            .map_or(0, |index| index as u64 + 1);
        index << STEP_SHIFT | u64::from(self.code.unwrap_or(0)) << CODE_SHIFT
    }

    /// The detail of `pack` inside packed capture facts; `None` without one
    /// or for a step a newer process knows and this one does not.
    pub fn unpack(bits: u64) -> Option<Self> {
        let index = usize::try_from((bits >> STEP_SHIFT) & 0xff).ok()?;
        let step = *FallbackStep::ALL.get(index.checked_sub(1)?)?;
        let code = (bits >> CODE_SHIFT) as u32;
        Some(Self {
            step,
            code: (code != 0).then_some(code),
        })
    }

    /// `DuplicateOutput 0x887A0004 DXGI_ERROR_UNSUPPORTED`.
    pub fn text(&self) -> String {
        match self.code {
            None => format!("{} (ohne Ergebniscode)", self.step.key()),
            Some(code) => match code_name(code) {
                Some(name) => format!("{} 0x{code:08X} {name}", self.step.key()),
                None => format!("{} 0x{code:08X}", self.step.key()),
            },
        }
    }
}

/// The name of a result code a capture can meet (`winerror.h`, `dxgi.h`).
pub fn code_name(code: u32) -> Option<&'static str> {
    Some(match code {
        0x887A_0001 => "DXGI_ERROR_INVALID_CALL",
        0x887A_0002 => "DXGI_ERROR_NOT_FOUND",
        0x887A_0004 => "DXGI_ERROR_UNSUPPORTED",
        0x887A_0005 => "DXGI_ERROR_DEVICE_REMOVED",
        0x887A_0006 => "DXGI_ERROR_DEVICE_HUNG",
        0x887A_0007 => "DXGI_ERROR_DEVICE_RESET",
        0x887A_0020 => "DXGI_ERROR_DRIVER_INTERNAL_ERROR",
        0x887A_0022 => "DXGI_ERROR_NOT_CURRENTLY_AVAILABLE",
        0x887A_0026 => "DXGI_ERROR_ACCESS_LOST",
        0x887A_0027 => "DXGI_ERROR_WAIT_TIMEOUT",
        0x887A_0028 => "DXGI_ERROR_SESSION_DISCONNECTED",
        0x887A_002B => "DXGI_ERROR_ACCESS_DENIED",
        0x8007_0005 => "E_ACCESSDENIED",
        0x8007_000E => "E_OUTOFMEMORY",
        0x8007_0057 => "E_INVALIDARG",
        0x8000_4002 => "E_NOINTERFACE",
        0x8000_4005 => "E_FAIL",
        _ => return None,
    })
}

/// `key` of the fallback cause with the detail's text, as the viewer reads it.
pub fn cause_text(key: &str, detail: Option<FallbackDetail>) -> String {
    match detail {
        Some(detail) => format!("{key}: {}", detail.text()),
        None => key.to_string(),
    }
}

/// Whether `cause` no longer holds for a capture on `path`: a Windows
/// fallback from Desktop Duplication ends when the capture is back on it
/// (the packed facts keep the last reported cause, `CaptureFacts::merged`).
pub fn fallback_ended(cause: CaptureFallback, path: Option<CapturePath>) -> bool {
    let windows = matches!(
        cause,
        CaptureFallback::DxgiUnsupported
            | CaptureFallback::DxgiAccessLost
            | CaptureFallback::DxgiAdapterChanged
            | CaptureFallback::DxgiFailed
    );
    windows && path == Some(CapturePath::DesktopDuplication)
}

/// The first try of the first path after a fallback.
pub const RETRY_FIRST: Duration = Duration::from_secs(2);
/// The longest wait between two tries (see the module text).
pub const RETRY_LONGEST: Duration = Duration::from_secs(60);

/// When a temporary fallback tries its first path again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPlan {
    next: Instant,
    wait: Duration,
}

impl RetryPlan {
    /// The plan of a fallback that happened at `now`.
    pub fn new(now: Instant) -> Self {
        Self {
            next: now + RETRY_FIRST,
            wait: RETRY_FIRST,
        }
    }

    pub fn due(&self, now: Instant) -> bool {
        now >= self.next
    }

    /// A try at `now` failed: the next one waits twice as long.
    pub fn failed(&mut self, now: Instant) {
        self.wait = self.wait.saturating_mul(2).min(RETRY_LONGEST);
        self.next = now + self.wait;
    }

    /// The wait before the next try.
    pub fn wait(&self) -> Duration {
        self.wait
    }
}

#[cfg(test)]
#[path = "capture_fallback_tests.rs"]
mod tests;
