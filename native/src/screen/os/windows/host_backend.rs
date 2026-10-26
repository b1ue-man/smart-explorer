//! The screen host of Windows (SCR-M4, U16-2 and U16-6): platform, monitors,
//! windows and limits are answered here, in the daemon; the source with its
//! capture opens in the host process (`os::host_worker_client`), which asks
//! this backend for it too. The test picture `synthetic` stays available
//! through the backend of SCR-M2. Input (SCR-M6): `inject` runs in the host
//! process that captures (`input_send`, decision of SEC on U16-4); the
//! daemon's `WorkerBackend` hands it there.
use std::io;

use super::capture_prepared::PreparedWindows;
use super::{capture_wgc, monitors, window_list};
use crate::screen::core::host_backend::{
    ScreenEncoder, ScreenHostBackend, ScreenSource, SourceRefusal,
};
use crate::screen::core::input_model::{InputCapabilities, InputEvent};
use crate::screen::core::negotiate::{CodecChoice, CodecOffer};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::window_list::{MonitorInfo, SourceId, WindowList};
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::core::wire_hello::HostPlatform;
use crate::screen::os::synthetic_backend::{self, SyntheticBackend};

/// Capture pauses on the secure desktop (UAC prompt, lock screen).
pub(super) const LIMIT_SECURE_DESKTOP_PAUSED: &str = "secure_desktop_paused";
/// A session shows one monitor or one window, never several.
pub(super) const LIMIT_ONE_MONITOR_PER_SESSION: &str = "one_monitor_per_session";
/// Windows draws a border around what Graphics Capture records.
pub(super) const LIMIT_CAPTURE_BORDER: &str = "capture_border";
/// Without Graphics Capture (before Windows 10 1903) no window is captured.
pub(super) const LIMIT_WINDOW_CAPTURE_UNSUPPORTED: &str = "window_capture_unsupported";
/// Without Graphics Capture the pointer is not part of the picture.
pub(super) const LIMIT_NO_CURSOR_CAPTURE: &str = "no_cursor_capture";

/// The limits of this host for `wgc` (Graphics Capture available or not).
pub(super) fn limits_for(wgc: bool) -> Vec<String> {
    let mut limits = vec![
        LIMIT_SECURE_DESKTOP_PAUSED,
        LIMIT_ONE_MONITOR_PER_SESSION,
        super::input_send::LIMIT_NO_CTRL_ALT_DEL,
        super::input_send::LIMIT_ELEVATED_WINDOWS_BLOCKED,
    ];
    if wgc {
        limits.push(LIMIT_CAPTURE_BORDER);
    } else {
        limits.push(LIMIT_WINDOW_CAPTURE_UNSUPPORTED);
        limits.push(LIMIT_NO_CURSOR_CAPTURE);
    }
    limits.into_iter().map(String::from).collect()
}

/// The host backend of Windows.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WindowsBackend;

impl WindowsBackend {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl ScreenHostBackend for WindowsBackend {
    fn memory_facts(&self)->Option<crate::screen::core::frame_memory::MemoryFacts> {super::session_memory::facts()}
    fn platform(&self) -> HostPlatform {
        HostPlatform::Windows
    }

    fn graphical_session(&self) -> bool {
        synthetic_backend::graphical_session()
    }

    fn monitors(&self) -> Vec<MonitorInfo> {
        monitors::infos()
    }

    fn encoders(&self) -> Vec<CodecOffer> {
        vec![CodecOffer::tiles()]
    }

    fn input(&self) -> InputCapabilities {
        InputCapabilities {
            pointer: true,
            keyboard: true,
            text: true,
            touch: true,
            actions: Vec::new(),
            toolbar_keys: crate::screen::os::input_keys::toolbar_keys(),
        }
    }

    fn limits(&self) -> Vec<String> {
        limits_for(capture_wgc::available())
    }

    fn windows(&self) -> io::Result<WindowList> {
        let mut list=window_list::list(self.limits());
        super::app_icons::attach(&mut list);
        Ok(list)
    }

    fn icon(&self, entry: crate::screen::core::app_icon::AuthorizedIconEntry<'_>) -> io::Result<Option<crate::screen::core::app_icon::IconReply>> {
        super::app_icons::load(entry)
    }

    fn prepare_session_source(&self, owner: &str, session: &str, source: &SourceId,
        wanted: Option<FrameSize>, control: bool,
        admission: std::sync::Arc<crate::screen::core::capture_admission::SessionAdmission>)
        -> Result<Box<dyn crate::screen::core::host_backend::PreparedSource>, SourceRefusal> {
        if matches!(source, SourceId::Synthetic) {
            return SyntheticBackend.prepare_session_source(owner, session, source, wanted, control, admission);
        }
        if !self.graphical_session() {return Err(SourceRefusal::new(super::capture_source::CAPTURE_UNAVAILABLE));}
        PreparedWindows::prepare(session, source, admission)
    }

    fn prepare_family_member(&self, owner: &str, session: &str,
        native: crate::remote_window::NativeWindowId,
        admission: std::sync::Arc<crate::screen::core::capture_admission::SessionAdmission>)
        -> Result<Box<dyn crate::screen::core::host_backend::PreparedSource>, SourceRefusal> {
        let handle = usize::try_from(native.0).map_err(|_|SourceRefusal::new(super::capture_source::WINDOW_NOT_FOUND))?;
        // This boundary can only prepare CreateForWindow. The metadata and
        // activation branches never fall back to a monitor or a desktop crop.
        self.prepare_session_source(owner, session, &SourceId::Window(handle.to_string()), None, false, admission)
    }

    fn open_source(&self, _source: &SourceId, _wanted: Option<FrameSize>)
        -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        Err(SourceRefusal::new("capture_grant_required"))
    }

    fn open_family_member(&self, _native: crate::remote_window::NativeWindowId)
        -> Result<Box<dyn ScreenSource>,SourceRefusal> {
        Err(SourceRefusal::new("capture_grant_required"))
    }

    fn open_encoder(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        SyntheticBackend.open_encoder(choice, size)
    }

    fn open_encoder_for_stage(&self, request: &crate::screen::core::host_stage::StageRequest) -> io::Result<Box<dyn ScreenEncoder>> {
        Ok(crate::screen::os::codec_inventory::registry().open_encoder_with_contract(&request.choice, request.size, request.fps, &[], request.frame_contract)?.encoder)
    }

    fn open_encoder_for_stage_admitted(
        &self,
        request: &crate::screen::core::host_stage::StageRequest,
        grant: crate::screen::core::capture_admission::CaptureGrant,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        Ok(crate::screen::os::codec_inventory::registry()
            .open_encoder_for_stage_admitted(request, &[], grant)?.encoder)
    }

    fn inject(&self, events: &[InputEvent]) -> Result<(), ControlBlock> {
        super::input_send::inject(events)
    }
    fn inject_family(&self,member:&crate::remote_window::CaptureMember,events:&[InputEvent])->Result<(),ControlBlock> {
        super::input_send::inject_family(member.geometry,events)
    }
    fn release_input(&self,events:&[InputEvent])->Result<(),ControlBlock> {
        super::input_send::release(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_limits_follow_graphics_capture() {
        assert_eq!(
            limits_for(true),
            [
                "secure_desktop_paused",
                "one_monitor_per_session",
                "no_ctrl_alt_del",
                "elevated_windows_blocked",
                "capture_border"
            ]
        );
        assert_eq!(
            limits_for(false),
            [
                "secure_desktop_paused",
                "one_monitor_per_session",
                "no_ctrl_alt_del",
                "elevated_windows_blocked",
                "window_capture_unsupported",
                "no_cursor_capture"
            ]
        );
        for limit in limits_for(true).into_iter().chain(limits_for(false)) {
            assert!(limit.len() <= 64, "{limit}");
        }
    }
}
