//! What the viewer asked of the host with `SET_MODE`, kept by the relay of a
//! viewer session so that a session built again after `session_changed`
//! (E48, `screen_ipc_reconnect.rs`) is put back as the viewer left it: the
//! source it switched to, the raise from the group view to the full view and
//! the picture size. The new session starts from the request of the first
//! opening only (`LinkDialer`), so without this memory those choices would
//! be lost.
//!
//! The host retains what a `SET_MODE` omits (`QualityMode::update`): a
//! source, and the size and frame rate of each stream kind on its own. The
//! memory folds every frame the same way and replays the result as at most
//! two frames.
use crate::screen::core::frame_size_caps::FrameSize;
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::{SetMode, StreamMode, ViewerFrame};

/// The explicit choices of one stream kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Choice {
    size: Option<FrameSize>,
    fps: Option<u16>,
}

impl Choice {
    fn is_set(&self) -> bool {
        self.size.is_some() || self.fps.is_some()
    }
}

/// One control frame of the viewer as the relay reads it (`relay_input`).
/// `fallback`: a `SET_MODE` that carries only an automatic fallback of the
/// viewer (`RELAY_VIEWER_FALLBACK`: the tiles after a decoder failure under
/// „Automatisch“). The host gets it as every frame; the memory keeps the
/// viewer's own `lossless`, so a session built again opens with the choice
/// instead of the fallback (round 11, N2).
#[derive(Clone, Debug)]
pub(crate) struct RelayedFrame {
    pub(crate) frame: ViewerFrame,
    pub(crate) fallback: bool,
}

/// The folded `SET_MODE` frames of one relayed session; empty until the
/// viewer sent one.
#[derive(Clone, Debug, Default)]
pub(crate) struct ModeMemory {
    last: Option<Folded>,
    /// The last `RATE_CEILING` of the viewer (BK6), replayed like a mode.
    ceiling: Option<Option<u64>>,
    /// The last `SET_DECODERS` of the viewer (P1), replayed like a mode.
    decoders: Option<crate::screen::core::set_decoders::SetDecoders>,
    /// The last `SET_DISPLAY` of the viewer (P9), replayed like a mode.
    display: Option<crate::screen::core::virtual_display::VirtualDisplayChoice>,
    /// The last `AUDIO` of the viewer (plan 2026-11-17 P9), replayed to a
    /// new session whose host answered `audio`.
    audio: Option<bool>,
    /// The viewer's last `CONTROL_REQUEST` (review 2026-11-19, Gemini 3/9):
    /// a released control stays released in the new session, a taken one
    /// is taken again.
    control: Option<crate::screen::core::wire_control::ControlAction>,
    /// The viewer's last `VISIBILITY` (Gemini 3/9): a hidden or partly
    /// covered view stays so in the new session.
    visibility: Option<crate::screen::core::visibility::ViewerVisibility>,
}

#[derive(Clone, Debug)]
struct Folded {
    mode: StreamMode,
    source: Option<SourceId>,
    preview: Choice,
    full: Choice,
    sharpness: bool,
    lossless: bool,
}

impl ModeMemory {
    /// Takes `relayed` in: as `note`, but a fallback leaves `lossless` as the
    /// viewer chose it last.
    pub(crate) fn note_relayed(&mut self, relayed: &RelayedFrame) {
        let kept = self.last.as_ref().map(|folded| folded.lossless);
        self.note(&relayed.frame);
        if relayed.fallback && matches!(relayed.frame, ViewerFrame::SetMode(_)) {
            if let Some(folded) = self.last.as_mut() {
                folded.lossless = kept.unwrap_or(false);
            }
        }
    }

    /// Takes `frame` in when it is a `SET_MODE`.
    pub(crate) fn note(&mut self, frame: &ViewerFrame) {
        if let ViewerFrame::RateCeiling(ceiling) = frame {
            self.ceiling = Some(*ceiling);
            return;
        }
        if let ViewerFrame::SetDecoders(set) = frame {
            self.decoders = Some(set.clone());
            return;
        }
        if let ViewerFrame::SetDisplay(choice) = frame {
            self.display = Some(*choice);
            return;
        }
        if let ViewerFrame::Audio(on) = frame {
            self.audio = Some(*on);
            return;
        }
        if let ViewerFrame::ControlRequest(request) = frame {
            self.control = Some(request.action);
            return;
        }
        if let ViewerFrame::Visibility(report) = frame {
            self.visibility = Some(report.clone());
            return;
        }
        let ViewerFrame::SetMode(set) = frame else {
            return;
        };
        let folded = self.last.get_or_insert(Folded {
            mode: set.mode,
            source: None,
            preview: Choice::default(),
            full: Choice::default(),
            sharpness: false,
            lossless: false,
        });
        folded.mode = set.mode;
        if set.source.is_some() {
            folded.source = set.source.clone();
        }
        let choice = match set.mode {
            StreamMode::Preview => &mut folded.preview,
            StreamMode::Full => &mut folded.full,
        };
        if set.size.is_some() {
            choice.size = set.size;
        }
        if set.fps.is_some() {
            choice.fps = set.fps;
        }
        folded.sharpness = set.sharpness;
        folded.lossless = set.lossless;
    }

    /// The ceiling a new session gets again; `None` when none was sent.
    pub(crate) fn rate_ceiling(&self) -> Option<Option<u64>> {
        self.ceiling
    }

    /// „Monitor“/„Auflösung“ a new session gets again; `None` when none was sent.
    pub(crate) fn display(
        &self,
    ) -> Option<crate::screen::core::virtual_display::VirtualDisplayChoice> {
        self.display
    }

    /// The viewer's last control request; `None` when it sent none.
    pub(crate) fn control(&self) -> Option<crate::screen::core::wire_control::ControlAction> {
        self.control
    }

    /// The viewer's last visibility report; `None` when it sent none.
    pub(crate) fn visibility(&self) -> Option<crate::screen::core::visibility::ViewerVisibility> {
        self.visibility.clone()
    }

    /// The viewer's last `AUDIO`; `None` when it sent none.
    pub(crate) fn audio(&self) -> Option<bool> {
        self.audio
    }

    /// The decoders a new session gets again; `None` when none were sent.
    pub(crate) fn decoders(&self) -> Option<crate::screen::core::set_decoders::SetDecoders> {
        self.decoders.clone()
    }

    /// The frames that put a new session where the viewer left the old one,
    /// in the order to send them; none when the viewer never sent a
    /// `SET_MODE`. The source travels once, in the first frame; the stream
    /// kind in force comes last.
    pub(crate) fn replay(&self) -> Vec<SetMode> {
        let Some(folded) = &self.last else {
            return Vec::new();
        };
        let (other_mode, other, current) = match folded.mode {
            StreamMode::Full => (StreamMode::Preview, folded.preview, folded.full),
            StreamMode::Preview => (StreamMode::Full, folded.full, folded.preview),
        };
        let mut source = folded.source.clone();
        let mut frame = |mode, choice: Choice| SetMode {
            mode,
            source: source.take(),
            size: choice.size,
            fps: choice.fps,
            sharpness: folded.sharpness,
            lossless: folded.lossless,
        };
        let mut frames = Vec::with_capacity(2);
        if other.is_set() {
            frames.push(frame(other_mode, other));
        }
        frames.push(frame(folded.mode, current));
        frames
    }
}
