//! Android as screen host (SCR-M8, C1 Android, C5): the screen host of the
//! share node in the app process. Capture (MediaProjection, the
//! accessibility service's stills, the app's own test picture) and H.264
//! (MediaCodec) run in the app; this backend names what the app reported
//! (`screen.hostPrereqs`), opens a bridge source per session
//! (`bridge_source.rs`) and hands the controlling viewer's input to the
//! accessibility service.
//!
//! Safeguards: every session with screen content waits for the system
//! dialog „Bildschirm teilen“ in the app (`consent_each_session`; still
//! previews need the accessibility service the user switched on); input
//! reaches the phone only through the accessibility service and only from
//! the viewer that holds control, which the session core grants with the
//! `ScreenControl` right; the indicator is the app's notification and the
//! system's status bar chip.
use std::io;
use std::sync::Arc;

use serde_json::json;

use super::bridge_channels::{self as channels, HostStart};
use super::bridge_source::BridgeSource;
use crate::screen::core::host_backend::{
    ScreenEncoder, ScreenHostBackend, ScreenSource, SessionIndicator, SourceRefusal,
    TilesScreenEncoder,
};
use crate::screen::core::input_model::{InputCapabilities, InputEvent, SystemAction};
use crate::screen::core::negotiate::{CodecChoice, CodecKind, CodecOffer, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::registry::ScreenRegistry;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::window_list::{AppEntry, ListSource, MonitorInfo, SourceId, WindowList};
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::core::wire_hello::{
    HostPlatform, SessionMode, LIMIT_CONSENT_EACH_SESSION, LIMIT_NO_KEY_EVENTS,
    LIMIT_STILL_MAX_3_PER_SECOND,
};

/// The platform limits of an Android host (C5, `HELLO_OK`).
pub(crate) const LIMITS: [&str; 3] = [
    LIMIT_CONSENT_EACH_SESSION,
    LIMIT_NO_KEY_EVENTS,
    LIMIT_STILL_MAX_3_PER_SECOND,
];
/// Reason of `no_display` while the app has not registered as host.
pub(crate) const HOST_APP_NOT_RUNNING: &str = "android_host_app_not_running";
/// Reason of `no_display` for an app window before Android 14 QPR2.
pub(crate) const APP_WINDOW_UNAVAILABLE: &str = "app_window_capture_unavailable";
/// The monitor id of the phone's display.
pub(crate) const DISPLAY_ID: &str = "0";

/// The screen host of the Android app.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BridgeBackend;

fn profile(key: &str) -> Option<H264Profile> {
    match key {
        "cb" => Some(H264Profile::ConstrainedBaseline),
        "high" => Some(H264Profile::High),
        _ => None,
    }
}

impl ScreenHostBackend for BridgeBackend {
    fn platform(&self) -> HostPlatform {
        HostPlatform::Android
    }

    fn graphical_session(&self) -> bool {
        channels::prereqs().registered
    }

    fn monitors(&self) -> Vec<MonitorInfo> {
        let Some((width, height)) = channels::prereqs().display else {
            return Vec::new();
        };
        vec![MonitorInfo {
            id: DISPLAY_ID.into(),
            name: "Bildschirm".into(),
            x: 0,
            y: 0,
            width,
            height,
            primary: true,
        }]
    }

    fn encoders(&self) -> Vec<CodecOffer> {
        let mut offers: Vec<CodecOffer> = channels::prereqs()
            .encoders
            .iter()
            .filter_map(|(key, hardware)| profile(key).map(|p| CodecOffer::h264(p, *hardware)))
            .collect();
        offers.push(CodecOffer::tiles());
        offers
    }

    fn input(&self) -> InputCapabilities {
        if !channels::prereqs().accessibility_on() {
            return InputCapabilities::default();
        }
        InputCapabilities {
            pointer: true,
            keyboard: false,
            text: true,
            touch: true,
            actions: vec![
                SystemAction::Back,
                SystemAction::Home,
                SystemAction::Recents,
                SystemAction::Notifications,
                SystemAction::QuickSettings,
                SystemAction::LockScreen,
                SystemAction::PowerDialog,
            ],
            toolbar_keys: Vec::new(),
        }
    }

    fn limits(&self) -> Vec<String> {
        LIMITS.iter().map(|limit| limit.to_string()).collect()
    }

    fn windows(&self) -> io::Result<WindowList> {
        let prereqs = channels::prereqs();
        let apps = prereqs
            .apps
            .iter()
            .map(|(package, label)| AppEntry {
                app_id: package.clone(),
                name: label.clone(),
                pid: None,
            })
            .collect();
        Ok(WindowList {
            windows: Vec::new(),
            apps,
            list_source: if prereqs.accessibility_on() {
                ListSource::AccessibilityWindows
            } else {
                ListSource::UsageStats
            },
            limits: self.limits(),
        })
    }

    fn open_source(
        &self,
        source: &SourceId,
        wanted: Option<FrameSize>,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        let prereqs = channels::prereqs();
        if !prereqs.registered {
            return Err(SourceRefusal::new(HOST_APP_NOT_RUNNING));
        }
        match source {
            SourceId::Monitor(id) if id != DISPLAY_ID => {
                return Err(SourceRefusal::new("unknown_monitor"))
            }
            SourceId::Window(_) if !prereqs.app_window_capture => {
                return Err(SourceRefusal::new(APP_WINDOW_UNAVAILABLE))
            }
            _ => {}
        }
        let display = prereqs.display.map(|(w, h)| FrameSize::new(w, h));
        let max = wanted.or(display).unwrap_or(FrameSize::new(1280, 720));
        let start = start_for(source, max);
        let size = display.unwrap_or(max);
        Ok(Box::new(BridgeSource::new(channels::open(start), size)))
    }

    fn open_encoder(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
    ) -> io::Result<Box<dyn ScreenEncoder>> {
        match choice.codec {
            CodecKind::Tiles => Ok(Box::new(TilesScreenEncoder::new(size)?)),
            CodecKind::H264 => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "H.264 runs in the app's MediaCodec encoder",
            )),
        }
    }

    fn inject(&self, events: &[InputEvent]) -> Result<(), ControlBlock> {
        if !channels::prereqs().accessibility_on() {
            return Err(ControlBlock::ControlUnavailable);
        }
        let streaming = channels::channels().iter().any(|channel| {
            matches!(channel.phase(), channels::Phase::Ready { .. }) && !channel.start.still
        });
        if !streaming {
            return Err(ControlBlock::ControlUnavailable);
        }
        channels::send_input(json!({"op": "input", "events": events}));
        Ok(())
    }
}

/// The start of a session for `source`: the viewer is the newest session of
/// the registry that waits for this source and has no channel yet, so the
/// app can name it before it asks its user.
fn start_for(source: &SourceId, max: FrameSize) -> HostStart {
    let key = source.to_string();
    let viewer = channels::registry().and_then(|registry| {
        let (_, sessions) = registry.sessions();
        sessions
            .into_iter()
            .filter(|session| {
                session.source == key
                    && matches!(
                        session.state,
                        SessionState::Starting | SessionState::WaitingForApproval
                    )
            })
            .max_by_key(|session| session.started_at)
    });
    HostStart {
        source: key,
        still: viewer
            .as_ref()
            .is_some_and(|session| session.mode == SessionMode::Preview),
        viewer_device_id: viewer
            .as_ref()
            .map(|s| s.device_id.clone())
            .unwrap_or_default(),
        viewer_name: viewer.map(|s| s.device_name).unwrap_or_default(),
        max_width: max.width,
        max_height: max.height,
    }
}

/// The host user's sign of a running session (C7): the app's notification
/// with „Beenden“ and, while MediaProjection runs, the system's chip. The
/// app shows it for every channel; without the app nothing is shown and
/// every session with screen content is refused.
#[derive(Clone, Debug)]
pub(crate) struct BridgeIndicator;

impl BridgeIndicator {
    /// The indicator of the sessions of `registry` (the share node's).
    pub(crate) fn new(registry: &Arc<ScreenRegistry>) -> Self {
        channels::set_registry(registry);
        Self
    }
}

impl SessionIndicator for BridgeIndicator {
    fn available(&self) -> bool {
        channels::prereqs().registered
    }

    fn show(&self) -> Result<(), String> {
        if self.available() {
            Ok(())
        } else {
            Err("Bildschirmfreigabe braucht die Smart-Explorer-App auf dem Telefon".into())
        }
    }
}
