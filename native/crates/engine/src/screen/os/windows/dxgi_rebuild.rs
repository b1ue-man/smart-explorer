//! Rebuilding a duplication (moved out of `capture_dxgi.rs` by the fix of
//! 2026-11-18, which keeps that file under 500 lines; plan 2026-11-17 P9
//! unchanged: a stale output is found again with a new device once per
//! streak of failures, every failure is logged once per streak).
//!
//! Fix of 2026-11-18 (host log of a Windows VM with an NVIDIA GPU): after
//! `AcquireNextFrame` reported a lost duplication, every rebuild was refused
//! with `E_INVALIDARG`, also on the output found again with a new device,
//! and the monitor fell back to Graphics Capture at once; a duplication
//! opened two seconds later succeeded. Now
//! - the old duplication is released and the deferred destruction of the
//!   device's released objects is submitted (`Flush`, Microsoft Learn
//!   `ID3D11DeviceContext::Flush`: Direct3D defers the destruction of
//!   objects) before the new `DuplicateOutput`;
//! - an `E_INVALIDARG` refusal settles (`capture_source::settling`): the
//!   capture pauses (`WouldBlock`, `CaptureRecovery::paused_by_duplication`)
//!   and the rebuild is tried again every `SETTLE_STEP` for
//!   `FALLBACK_AFTER`; only then the error goes out and the monitor falls
//!   back (with the backoff of `capture_monitor.rs`);
//! - the "found again" line names the adapter of the old and the new
//!   output, so the host log tells a moved output from the same one.
use std::io;
use std::time::Instant;

use super::prepared::{duplicate, stale, Failing};
use super::{DxgiCapture, DxgiFailure, PreparedDxgi};
use crate::screen::core::capture_fallback::FallbackStep;
use crate::screen::os::capture_source::{self, SETTLE_STEP};

/// The last error of a rebuild that waits for its next try (`RebuildBackoff`):
/// the same result code, or the same kind without one.
fn repeated(failing: &Failing) -> io::Error {
    match failing.code {
        Some(code) => io::Error::other(windows::core::Error::from_hresult(code)),
        None => io::Error::new(
            failing.kind,
            "screen capture rebuild waits after repeated failures",
        ),
    }
}

/// The error a settling rebuild pauses with.
fn settle_wait() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        "DXGI duplication refused with E_INVALIDARG; tried again while it settles",
    )
}

impl DxgiCapture {
    pub(super) fn note_failure(&mut self, failure: &DxgiFailure) {
        let code = capture_source::result_code(&failure.error);
        // A streak of the same failure is logged once.
        if self
            .failing
            .is_none_or(|failing| failing.step != failure.step || failing.code != code)
        {
            capture_source::log_failure(failure.step, &failure.error);
        }
        self.failing = Some(Failing {
            since: self
                .failing
                .map_or_else(Instant::now, |failing| failing.since),
            step: failure.step,
            kind: failure.error.kind(),
            code,
        });
    }

    /// The current streak of failures while it settles.
    fn settles(&self) -> bool {
        self.failing.is_some_and(|failing| {
            capture_source::settling(failing.step, failing.code, failing.since.elapsed())
        })
    }

    /// `Grabber::reopen`: one rebuild, or while a refusal settles at most
    /// one every `SETTLE_STEP` (a pause in between).
    pub(super) fn reopen_settling(&mut self) -> io::Result<()> {
        if self.settles() && self.tried.is_some_and(|at| at.elapsed() < SETTLE_STEP) {
            return Err(settle_wait());
        }
        // Measurement of 2026-11-18 (a locked PC: a rebuild and a log line
        // about every 200 ms): past its settling time a rebuild that keeps
        // failing is tried after 1 s, 2 s, 4 s, then every 5 s
        // (`RebuildBackoff`); in between its last error stands, so the
        // fallback to Graphics Capture sees what it saw before.
        if let Some(failing) = self.failing.filter(|_| !self.settles()) {
            if !self.backoff.due(Instant::now()) {
                return Err(repeated(&failing));
            }
        }
        self.tried = Some(Instant::now());
        match self.rebuild() {
            Ok(()) => {
                let spaced = self.backoff.succeeded();
                if let Some(failing) = self.failing {
                    eprintln!(
                        "screen capture: duplication rebuilt after {} ms of {} failures \
                         ({spaced} spaced rebuilds)",
                        failing.since.elapsed().as_millis(),
                        failing.step.key()
                    );
                }
                self.failing = None;
                self.reattached = false;
                self.tried = None;
                Ok(())
            }
            Err(failure) => {
                self.note_failure(&failure);
                if self.settles() {
                    return Err(settle_wait());
                }
                self.backoff.failed(Instant::now());
                Err(failure.error)
            }
        }
    }

    fn rebuild(&mut self) -> Result<(), DxgiFailure> {
        self.recovery.request(true);
        self.stop();
        // The old duplication and the objects that referenced its surface
        // are released above; their deferred destruction is submitted now.
        // SAFETY: the live immediate context of the capture's device.
        unsafe { self.prepared.devices.context.Flush() };
        match self.duplicate_again() {
            Err(failure) if stale(&failure) && !self.reattached => {
                // P9: the output found again, with a device on its adapter,
                // once per streak of failures.
                self.reattached = true;
                capture_source::log_failure(failure.step, &failure.error);
                let prepared = PreparedDxgi::prepare(&self.prepared.device)
                    .map_err(DxgiFailure::at(FallbackStep::DeviceCreate))?;
                eprintln!(
                    "screen capture: output {} found again with a new device on {} (before: {})",
                    self.prepared.device, prepared.adapter, self.prepared.adapter
                );
                self.prepared = prepared;
                self.duplicate_again()
            }
            result => result,
        }
    }

    fn duplicate_again(&mut self) -> Result<(), DxgiFailure> {
        let geometry = self
            .prepared
            .geometry()
            .map_err(DxgiFailure::at(FallbackStep::OutputCheck))?;
        self.budget
            .replace(geometry)
            .map_err(DxgiFailure::at(FallbackStep::CaptureMemory))?;
        // Gemini round 3, 13: a refused duplication gives its new grant back
        // at once instead of holding it until the next try.
        let live = duplicate(&self.prepared, &self.budget, self.follow_desktop)
            .inspect_err(|_| self.budget.suspend())?;
        self.live = Some(live);
        self.size = geometry.displayed();
        Ok(())
    }
}
