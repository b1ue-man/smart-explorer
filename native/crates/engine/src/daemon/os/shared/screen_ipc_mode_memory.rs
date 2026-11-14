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

/// The folded `SET_MODE` frames of one relayed session; empty until the
/// viewer sent one.
#[derive(Clone, Debug, Default)]
pub(crate) struct ModeMemory {
    last: Option<Folded>,
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
    /// Takes `frame` in when it is a `SET_MODE`.
    pub(crate) fn note(&mut self, frame: &ViewerFrame) {
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
