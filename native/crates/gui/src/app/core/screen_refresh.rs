//! The frame rate the desktop viewer asks a host for (plan 2026-11-14
//! „Bildkette“ B-01): the refresh rate of the monitor its window is on, as
//! the system reports it, instead of a fixed 60. eframe 0.29 does not hand
//! the refresh rate out (`egui::ViewportInfo` has the monitor size only), so
//! the platform adapter asks the system: Windows `EnumDisplaySettingsW` for
//! the monitor of the viewer's window, X11 the RandR mode of the CRTC under
//! the active window, Wayland the current `wl_output` mode.
#[cfg(windows)]
#[path = "../os/windows/screen_refresh.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "../os/linux_os/screen_refresh.rs"]
mod platform;
#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    pub fn refresh_millihertz() -> Option<u32> {
        None
    }
}

/// The rate a v0.5.196 viewer always asked for; used only when the system
/// reports no refresh rate, so such a viewer behaves as before.
pub const FALLBACK_FPS: u16 = 60;

/// Whole frames per second for a refresh rate in mHz, rounded up so a
/// 59.94 Hz or 143.86 Hz mode asks for 60 or 144; `FALLBACK_FPS` without one.
pub fn fps_from_millihertz(refresh: Option<u32>) -> u16 {
    refresh
        .filter(|millihertz| *millihertz > 0)
        .map(|millihertz| u16::try_from(millihertz.div_ceil(1000)).unwrap_or(u16::MAX))
        .unwrap_or(FALLBACK_FPS)
}

/// How long an answer of the system counts (Gemini 3/23: every `SET_MODE`
/// opened a new X11/Wayland connection on the window's thread); a window
/// moved to another monitor is asked again after it.
const ASKED_FOR: std::time::Duration = std::time::Duration::from_secs(2);

/// The rate the full view asks for now.
pub fn wanted_fps() -> u16 {
    use std::sync::{Mutex, PoisonError};
    static KNOWN: Mutex<Option<(std::time::Instant, u16)>> = Mutex::new(None);
    let mut known = KNOWN.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((at, fps)) = *known {
        if at.elapsed() < ASKED_FOR {
            return fps;
        }
    }
    let fps = fps_from_millihertz(platform::refresh_millihertz());
    *known = Some((std::time::Instant::now(), fps));
    fps
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B-01: the viewer asks for its display's refresh rate, not 60.
    #[test]
    fn bildkette_task_rate_viewer_asks_for_the_display_refresh_rate() {
        assert_eq!(fps_from_millihertz(Some(59_940)), 60);
        assert_eq!(fps_from_millihertz(Some(143_856)), 144);
        assert_eq!(fps_from_millihertz(Some(240_000)), 240);
        assert_eq!(fps_from_millihertz(Some(360_000)), 360);
        assert_eq!(fps_from_millihertz(None), FALLBACK_FPS);
        assert_eq!(fps_from_millihertz(Some(0)), FALLBACK_FPS);
        let wanted = wanted_fps();
        assert!(wanted >= 1);
        let request = super::super::screen_pool::full_request(
            &[],
            None,
            crate::screen::core::pixel_convert::FrameSize::new(1920, 1080),
            false,
        );
        assert_eq!(request.max_fps, wanted);
        let pool = include_str!("screen_pool.rs");
        assert!(!pool.contains("FULL_FPS"));
    }
}
