//! The first picture of a desktop duplication (fix of 2026-11-17: a Windows
//! viewer stayed black while the Windows host showed its indicator).
//!
//! Two ways left a still screen without any picture, with no error that
//! would switch to Graphics Capture:
//! - A frame without a reference picture was read even when DXGI had not
//!   updated the desktop image (`LastPresentTime` 0, only the pointer
//!   changed). The content of such a surface is not specified
//!   (`DXGI_OUTDUPL_FRAME_INFO`); WebRTC copies only with
//!   `AccumulatedFrames > 0`, Sunshine only with `LastPresentTime != 0`.
//! - A read skipped for want of an admitted image (`WouldBlock`) left no
//!   reference, and on a still screen every later wait timed out, so the
//!   change that triggered the read was never delivered.
//!
//! Without a reference picture a timed-out wait therefore rebuilds the
//! duplication: the first frame of a new duplication carries the whole
//! desktop. After `BLIND_LIMIT` rebuilds without an updated frame any frame
//! is read again (the behaviour before the fix), so a desktop that DXGI
//! never reports as updated still shows.
use windows::Win32::Graphics::Dxgi::DXGI_OUTDUPL_FRAME_INFO;

/// Rebuilds without an updated frame before any frame is read.
const BLIND_LIMIT: u8 = 3;

#[derive(Default)]
pub(super) struct FirstPicture {
    /// Rebuilds since the last delivered picture.
    blind: u8,
    /// A read was skipped since the last delivered picture.
    missed: bool,
}

impl FirstPicture {
    /// Whether a frame may become the reference picture.
    pub(super) fn readable(&self, info: &DXGI_OUTDUPL_FRAME_INFO) -> bool {
        updated(info) || self.blind >= BLIND_LIMIT
    }

    pub(super) fn missed(&mut self) {
        self.missed = true;
    }

    pub(super) fn delivered(&mut self) {
        *self = Self::default();
    }

    /// On a timed-out wait without a reference picture: whether to rebuild
    /// the duplication now.
    pub(super) fn rebuild(&mut self) -> bool {
        if !self.missed && self.blind >= BLIND_LIMIT {
            return false;
        }
        self.missed = false;
        self.blind = self.blind.saturating_add(1);
        true
    }
}

/// DXGI updated the desktop image in this frame, not only the pointer.
fn updated(info: &DXGI_OUTDUPL_FRAME_INFO) -> bool {
    info.LastPresentTime != 0 && info.AccumulatedFrames > 0
}
