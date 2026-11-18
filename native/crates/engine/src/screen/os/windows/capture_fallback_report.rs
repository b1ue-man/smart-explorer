//! Why a monitor left Desktop Duplication and what the session of this host
//! process captures with (moved out of `capture_source.rs` for plan
//! 2026-11-17 P9, which keeps that file under 500 lines).
//!
//! P9 adds the failed step and its result code (`FallbackDetail`): every
//! failure of a duplication and every change of path or fallback goes to the
//! host process's standard error, which the daemon writes to the screen host
//! log (`host_worker_log.rs`), and the detail of the fallback goes with the
//! capture facts to the viewer (`fallback_bits`).
use std::io;
use std::sync::Mutex;
use std::time::Duration;

use windows::core::HRESULT;
use windows::Win32::Foundation::{E_ACCESSDENIED, E_INVALIDARG};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_NOT_CURRENTLY_AVAILABLE, DXGI_ERROR_SESSION_DISCONNECTED, DXGI_ERROR_UNSUPPORTED,
};

use super::{CapturePath, FIRST_FRAME_WAIT};
use crate::screen::core::capture_fallback::{FallbackDetail, FallbackStep};

/// Why a monitor left Desktop Duplication for Graphics Capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::screen::os) enum FallbackCause {
    /// `DXGI_ERROR_UNSUPPORTED` or `DXGI_ERROR_NOT_CURRENTLY_AVAILABLE`.
    Unsupported,
    /// `DXGI_ERROR_ACCESS_LOST`, and the new duplication failed.
    AccessLost,
    /// The output is no longer the one of its adapter (no result code).
    AdapterChanged,
    /// Any other refused duplication.
    Failed,
}

impl FallbackCause {
    pub(in crate::screen::os) fn key(self) -> &'static str {
        match self {
            Self::Unsupported => "dxgi_unsupported",
            Self::AccessLost => "dxgi_access_lost",
            Self::AdapterChanged => "dxgi_adapter_changed",
            Self::Failed => "dxgi_failed",
        }
    }
}

/// How long a duplication refused with a documented passing result
/// (`DXGI_ERROR_UNSUPPORTED` during a desktop or mode switch) is retried
/// before Graphics Capture takes over [P: the first-picture wait].
pub(in crate::screen::os) const FALLBACK_AFTER: Duration = FIRST_FRAME_WAIT;

/// The fallback a duplication that has been failing for `failing_for` with
/// `code` (`None`: an error without result code) asks for; `lost` when the
/// duplication was lost before. The secure desktop (`E_ACCESSDENIED`) and an
/// inactive session pause instead, as before.
pub(in crate::screen::os) fn fallback_cause(
    code: Option<HRESULT>,
    lost: bool,
    failing_for: Duration,
) -> Option<FallbackCause> {
    let Some(code) = code else {
        return Some(FallbackCause::AdapterChanged);
    };
    if code == E_ACCESSDENIED || code == DXGI_ERROR_SESSION_DISCONNECTED {
        return None;
    }
    let lost_or = |cause| {
        Some(if lost {
            FallbackCause::AccessLost
        } else {
            cause
        })
    };
    if code == DXGI_ERROR_UNSUPPORTED || code == DXGI_ERROR_NOT_CURRENTLY_AVAILABLE {
        return if failing_for >= FALLBACK_AFTER {
            lost_or(FallbackCause::Unsupported)
        } else {
            None
        };
    }
    // Fix of 2026-11-18: `E_INVALIDARG` is retried as long as the passing
    // results before it falls back (`settling`).
    if code == E_INVALIDARG && failing_for < FALLBACK_AFTER {
        return None;
    }
    lost_or(FallbackCause::Failed)
}

/// How far apart a settling rebuild (`settling`) is tried again.
pub(in crate::screen::os) const SETTLE_STEP: Duration = Duration::from_millis(100);

/// Fix of 2026-11-18 (host log of a Windows VM): whether a rebuild that has
/// been failing for `failing_for` in `step` with `code` waits and is tried
/// again (every `SETTLE_STEP`) instead of falling back at once.
/// `IDXGIOutput1::DuplicateOutput` documents `E_INVALIDARG` for a device
/// that is invalid or of another adapter and for "the calling application is
/// already duplicating this desktop output" (Microsoft Learn,
/// `IDXGIOutput1::DuplicateOutput`, return values). After a lost duplication
/// the rebuild is refused that way although the device is created on the
/// output's own adapter from an `IDXGIFactory1` (`dxgi_output.rs`,
/// `d3d_device::create_on`), also with the output found again and a new
/// device, while a duplication opened two seconds later succeeds: the
/// release of the old duplication had not finished [assumption
/// A-SHADOWHOST-SETTLE: the release completes within `FALLBACK_AFTER`; if
/// not, Graphics Capture takes over as before, with the backoff of
/// `capture_monitor.rs`]. At activation `fallback_cause` is asked with
/// `FALLBACK_AFTER` and falls back at once, as before.
pub(in crate::screen::os) fn settling(
    step: FallbackStep,
    code: Option<HRESULT>,
    failing_for: Duration,
) -> bool {
    step == FallbackStep::DuplicateOutput
        && code == Some(E_INVALIDARG)
        && failing_for < FALLBACK_AFTER
}

/// P9: the fallback of a rebuild that failed in `step` with `error`. Only a
/// changed output (`NotFound` without result code, after the output was
/// searched again) is `AdapterChanged`; capture memory still held by older
/// pictures (`WouldBlock`) waits for their release, and any other error
/// without a result code (a mode switch between two descriptions) is
/// retried for `FALLBACK_AFTER` before it falls back as `Failed`.
pub(in crate::screen::os) fn rebuild_fallback(
    step: FallbackStep,
    kind: io::ErrorKind,
    code: Option<HRESULT>,
    lost: bool,
    failing_for: Duration,
) -> Option<FallbackCause> {
    if code.is_some() {
        return fallback_cause(code, lost, failing_for);
    }
    if kind == io::ErrorKind::WouldBlock || step == FallbackStep::CaptureMemory {
        return None;
    }
    if kind == io::ErrorKind::NotFound {
        return Some(FallbackCause::AdapterChanged);
    }
    (failing_for >= FALLBACK_AFTER).then_some(if lost {
        FallbackCause::AccessLost
    } else {
        FallbackCause::Failed
    })
}

/// The Windows result code inside an I/O error, if there is one.
pub(in crate::screen::os) fn result_code(error: &io::Error) -> Option<HRESULT> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<windows::core::Error>())
        .map(windows::core::Error::code)
}

/// The detail of a failure of `step` with `error`.
pub(in crate::screen::os) fn detail_of(step: FallbackStep, error: &io::Error) -> FallbackDetail {
    FallbackDetail {
        step,
        code: result_code(error).map(|code| code.0 as u32),
    }
}

/// One failed step of a capture in the host log (standard error).
pub(in crate::screen::os) fn log_failure(step: FallbackStep, error: &io::Error) {
    eprintln!(
        "screen capture: {} failed: {error}",
        detail_of(step, error).text()
    );
}

/// What the session of this host process captures with (one process serves
/// one session, like the pointer overlay).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::screen::os) struct CaptureReport {
    pub path: Option<CapturePath>,
    /// Graphics Capture cannot limit its delivery to the frame interval.
    pub rate_capped: bool,
    pub fallback: Option<FallbackCause>,
    /// P9: the failed step and result code of `fallback`.
    pub detail: Option<FallbackDetail>,
}

impl CaptureReport {
    /// The session's limits in the strings of `host_backend.rs`:
    /// `capture_border` only under Graphics Capture, `capture_rate_capped`
    /// when its rate could not be set.
    pub(in crate::screen::os) fn limits(&self) -> Vec<&'static str> {
        let mut limits = Vec::new();
        if self.path == Some(CapturePath::GraphicsCapture) {
            limits.push(super::super::host_backend::LIMIT_CAPTURE_BORDER);
            if self.rate_capped {
                limits.push(super::super::host_backend::LIMIT_CAPTURE_RATE_CAPPED);
            }
        }
        limits
    }
}

static REPORT: Mutex<CaptureReport> = Mutex::new(CaptureReport {
    path: None,
    rate_capped: false,
    fallback: None,
    detail: None,
});

/// The report of this process's session (error output, chain facts, tests).
pub(in crate::screen::os) fn report() -> CaptureReport {
    *REPORT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Changes the report; a changed path, limit or fallback goes to the host
/// process's error output (the screen host log).
pub(in crate::screen::os) fn update_report(change: impl FnOnce(&mut CaptureReport)) {
    let mut report = REPORT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = *report;
    change(&mut report);
    if *report != before {
        eprintln!(
            "screen capture: path {}, limits [{}], fallback {}",
            report.path.map_or("none", CapturePath::key),
            report.limits().join(","),
            report.fallback.map_or_else(
                || "none".to_string(),
                |cause| crate::screen::core::capture_fallback::cause_text(
                    cause.key(),
                    report.detail
                )
            )
        );
    }
}

/// The detail of the current fallback in the bits of the packed capture
/// facts that `CaptureFacts` leaves free (`FallbackDetail::pack`).
pub(in crate::screen::os) fn fallback_bits() -> u64 {
    let report = report();
    match (report.fallback, report.detail) {
        (Some(_), Some(detail)) => detail.pack(),
        _ => 0,
    }
}
