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
use crate::screen::core::host_backend::{
    ScreenEncoder, ScreenHostBackend, ScreenSource, SessionIndicator, SourceRefusal,
    TilesScreenEncoder,
};
use crate::screen::core::input_model::{InputCapabilities, InputEvent, SystemAction};
use crate::screen::core::negotiate::{CodecChoice, CodecKind, CodecOffer};
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
pub(crate) const LIMITS: [&str; 4] = [
    LIMIT_CONSENT_EACH_SESSION,
    LIMIT_NO_KEY_EVENTS,
    LIMIT_STILL_MAX_3_PER_SECOND,
    "android_system_selected_source",
];
/// Reason of `no_display` while the app has not registered as host.
pub(crate) const HOST_APP_NOT_RUNNING: &str = "android_host_app_not_running";
/// Public MediaProjection callbacks cannot prove the selected package identity.
pub(crate) const APP_WINDOW_UNAVAILABLE: &str = "source_identity_unavailable";
/// The monitor id of the phone's display.
pub(crate) const DISPLAY_ID: &str = "0";

/// The screen host of the Android app.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BridgeBackend;


impl ScreenHostBackend for BridgeBackend {
    fn session_memory_ceiling(&self)->u64 {
        channels::prereqs().working_bytes.unwrap_or(crate::screen::core::frame_budget::MAX_WORKING_BYTES)
            .min(crate::screen::core::frame_budget::MAX_WORKING_BYTES)
    }
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
            name: "Android-Systemauswahl".into(),
            x: 0,
            y: 0,
            width,
            height,
            primary: true,
        }]
    }

    fn encoders(&self) -> Vec<CodecOffer> {
        let mut offers = channels::prereqs().encoders;
        offers.truncate(crate::screen::core::negotiate::MAX_CODEC_OFFERS-1);
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
                icon: super::android_app_icons::reference(package),
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

    fn icon(&self, entry: crate::screen::core::app_icon::AuthorizedIconEntry<'_>) -> io::Result<Option<crate::screen::core::app_icon::IconReply>> {
        super::android_app_icons::load(entry)
    }

    fn prepare_session_source(&self, _owner:&str, session:&str, source:&SourceId,
        wanted:Option<FrameSize>, _control:bool,
        admission:Arc<crate::screen::core::capture_admission::SessionAdmission>)
        ->Result<Box<dyn crate::screen::core::host_backend::PreparedSource>,SourceRefusal> {
        let prereqs = channels::prereqs();
        if !prereqs.registered {
            return Err(SourceRefusal::new(HOST_APP_NOT_RUNNING));
        }
        match source {
            SourceId::Monitor(id) if id != DISPLAY_ID => {
                return Err(SourceRefusal::new("unknown_monitor"))
            }
            SourceId::Window(_) => {
                return Err(SourceRefusal::new(APP_WINDOW_UNAVAILABLE))
            }
            _ => {}
        }
        let display = prereqs.display.map(|(w, h)| FrameSize::new(w, h));
        let max = wanted.or(display).unwrap_or(FrameSize::new(0, 0));
        let start = start_for(source, max);
        let key=crate::screen::core::capture_admission::SourceKey::new(session,&source.to_string())
            .map_err(|e|SourceRefusal::new(&e.to_string()))?;
        let channel=channels::open(start);
        channel.capture.attach(admission,key.clone()).map_err(|e|SourceRefusal::new(&e.to_string()))?;
        let limits=crate::screen::core::frame_budget::FrameLimits::RESOURCE;
        Ok(Box::new(super::android_capture_prepared::AndroidPrepared::new(channel,key,limits)))
    }
    fn open_source(&self,_source:&SourceId,_wanted:Option<FrameSize>)->Result<Box<dyn ScreenSource>,SourceRefusal> {
        Err(SourceRefusal::new(if channels::prereqs().registered {"capture_grant_required"} else {HOST_APP_NOT_RUNNING}))
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

    fn open_encoder_for_stage_admitted(&self,request:&crate::screen::core::host_stage::StageRequest,
        grant:crate::screen::core::capture_admission::CaptureGrant)->io::Result<Box<dyn ScreenEncoder>> {
        if request.choice.codec != CodecKind::Tiles {return Err(io::Error::new(io::ErrorKind::Unsupported,
            "H.264 runs in the app's admitted MediaCodec encoder"));}
        let limits=request.frame_contract.map_or(crate::screen::core::frame_budget::FrameLimits::LEGACY,|c|c.limits);
        let encoder=Box::new(crate::screen::core::host_codec_adapters::AdmittedTilesEncoder::new(request.size,limits,grant.clone())?);
        crate::screen::core::host_hybrid::wrap_captured(encoder,request,grant)
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
