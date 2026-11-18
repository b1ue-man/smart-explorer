//! A monitor on Desktop Duplication that gives way to Graphics Capture once
//! the duplication asks for it (moved out of `capture_prepared.rs` for plan
//! 2026-11-17 P9, which keeps that file under 500 lines).
//!
//! The duplication asks for the fallback (`Grabber::fallback_cause`) when it
//! is unsupported for longer than `FALLBACK_AFTER`, lost and not rebuilt, or
//! its output left the adapter. The session continues with a new
//! generation, so the host process starts its encoder anew.
//!
//! P9: the fallback is temporary. While on Graphics Capture the monitor
//! tries Desktop Duplication again by `RetryPlan` (after 2 s, then each wait
//! twice as long, at most a minute apart): the fallback and its grant end,
//! the duplication asks for its own plan and grant (`first`), and when it
//! opens the report forgets the fallback; when it does not, Graphics Capture
//! opens again with the generation after the failed try. Every switch and
//! its reason (step and result code, `FallbackDetail`) goes to the host log
//! and with the report to the viewer.
//!
//! Fix of 2026-11-18: a fallback soon after Desktop Duplication came back
//! continues the retry plan with a doubled wait (`next_plan`) instead of
//! starting at 2 s again; the host log of a Windows VM showed the monitor
//! switching between both paths every few seconds.
use std::io;
use std::time::{Duration, Instant};

use crate::screen::core::capture_admission::CaptureGrant;
use crate::screen::core::capture_fallback::{RetryPlan, RETRY_LONGEST};
use crate::screen::core::capture_picture::SourcePicture;
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::host_backend::SourceFrame;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::os::capture_source::{self, CapturePath, Grabber};

/// Opens a capture of the monitor with the generation after the given one.
pub(in crate::screen::os) type OpenFallback =
    Box<dyn FnMut(u64) -> io::Result<Box<dyn Grabber>> + Send>;

pub(in crate::screen::os) struct MonitorGrabber {
    pub(in crate::screen::os) current: Box<dyn Grabber>,
    /// Graphics Capture of the monitor.
    pub(in crate::screen::os) fallback: Option<OpenFallback>,
    /// P9: Desktop Duplication of the monitor again.
    pub(in crate::screen::os) first: Option<OpenFallback>,
    /// P9: while on the fallback, when the first path is tried again.
    pub(in crate::screen::os) retry: Option<RetryPlan>,
    /// Fix of 2026-11-18: since when the first path is back and the plan
    /// that brought it back (`next_plan`).
    pub(in crate::screen::os) back: Option<(Instant, RetryPlan)>,
}

/// Fix of 2026-11-18 (host log of a Windows VM): how long Desktop
/// Duplication must hold after it came back before a new fallback starts
/// its retries at `RETRY_FIRST` again: as long as the longest wait between
/// two tries [assumption A-SHADOWHOST-STABLE].
pub(in crate::screen::os) const STABLE_AFTER: Duration = RETRY_LONGEST;

/// The retry plan of a fallback at `now` (pure, for the task tests). The
/// first path came back at `back.0` with the plan `back.1`: a fallback
/// before `STABLE_AFTER` continues that plan with a doubled wait, so a path
/// that comes back and fails again is tried after 2, 4, 8 … 60 s instead of
/// every 2 s; a first path that held longer starts a new plan.
pub(in crate::screen::os) fn next_plan(
    back: Option<(Instant, RetryPlan)>,
    now: Instant,
) -> RetryPlan {
    match back {
        Some((since, mut plan)) if now.saturating_duration_since(since) < STABLE_AFTER => {
            plan.failed(now);
            plan
        }
        _ => RetryPlan::new(now),
    }
}

impl MonitorGrabber {
    /// Switches when the duplication asks for it; true after a switch.
    fn fall_back(&mut self) -> io::Result<bool> {
        let Some(cause) = self.current.fallback_cause() else {
            return Ok(false);
        };
        let detail = self.current.fallback_detail();
        let Some(open) = self.fallback.as_mut() else {
            return Ok(false);
        };
        let previous = self.current.reference_generation();
        // The duplication and its grant end before Graphics Capture asks for
        // its own grant.
        self.current = Box::new(Retired {
            size: self.current.size(),
            generation: previous,
        });
        if self.first.is_some() {
            self.retry = Some(next_plan(self.back.take(), Instant::now()));
        }
        capture_source::update_report(|report| {
            report.fallback = Some(cause);
            report.detail = detail;
        });
        self.current = open(previous)?;
        eprintln!(
            "screen capture: Desktop Duplication -> Graphics Capture ({}); retry in {} s",
            crate::screen::core::capture_fallback::cause_text(cause.key(), detail),
            self.retry.map_or(0, |plan| plan.wait().as_secs())
        );
        Ok(true)
    }

    /// P9: tries Desktop Duplication again when its retry is due; true when
    /// the capture changed (either way).
    fn try_first(&mut self) -> bool {
        let now = Instant::now();
        if !self.retry.is_some_and(|plan| plan.due(now)) {
            return false;
        }
        let (Some(first), Some(plan)) = (self.first.as_mut(), self.retry.as_mut()) else {
            return false;
        };
        let previous = self.current.reference_generation();
        // The fallback and its grant end before the duplication asks for its
        // own grant.
        self.current = Box::new(Retired {
            size: self.current.size(),
            generation: previous,
        });
        match first(previous) {
            Ok(grabber) => {
                self.current = grabber;
                self.back = self.retry.take().map(|plan| (now, plan));
                capture_source::update_report(|report| {
                    report.fallback = None;
                    report.detail = None;
                });
                eprintln!("screen capture: back on Desktop Duplication");
                return true;
            }
            Err(error) => {
                plan.failed(now);
                eprintln!(
                    "screen capture: Desktop Duplication still refused ({error}); next try in {} s",
                    plan.wait().as_secs()
                );
            }
        }
        // The try may have used the next generation: one after it.
        let Some(open) = self.fallback.as_mut() else {
            return true;
        };
        match open(previous.saturating_add(1)) {
            Ok(grabber) => self.current = grabber,
            Err(error) => {
                eprintln!("screen capture: Graphics Capture did not open again: {error}");
                self.current = Box::new(Retired {
                    size: self.current.size(),
                    generation: previous.saturating_add(1),
                });
            }
        }
        true
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
        if self.try_first() {
            return Ok(SourcePicture::Unchanged);
        }
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
        if self.try_first() {
            return Ok(SourcePicture::Unchanged);
        }
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
        crate::screen::os::capture_prepared::recovery::CaptureRecovery::paused_by_duplication(error)
            .map(|_| ())
    }
}

/// The place of a capture between two paths, or of a fallback that could
/// not open; it keeps the generation, which never goes back.
struct Retired {
    size: FrameSize,
    generation: u64,
}

impl Grabber for Retired {
    fn size(&self) -> FrameSize {
        self.size
    }
    fn reference_generation(&self) -> u64 {
        self.generation
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

#[cfg(test)]
#[path = "shadowhost_capture_task_tests.rs"]
mod shadowhost_capture_task_tests;
