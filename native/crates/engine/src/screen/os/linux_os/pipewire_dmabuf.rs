//! DMA-BUF capture of a Wayland screen cast (plan 2026-11-14, package
//! BK8b, B-15): the decision DMA-BUF or shared memory, the negotiation state
//! of one stream with its fallback, the slot of the newest held DMA-BUF
//! picture and the rule "unchanged" without a read-back. The PipeWire calls
//! are in `pipewire_dmabuf_stream.rs`; this file has no FFI.
//!
//! Decisions (each documented in the BK8b report):
//! - DMA-BUF only when the GPU encoder imports the buffer (VA-API with video
//!   processor, `linux_hw_codecs::dmabuf_import_target`); a software coder,
//!   the tiles, NVENC or a pointer the host must mix into the picture keep
//!   or return to shared memory (B-15: else a read-back would follow).
//! - A failed modifier choice, layout, import or conversion ends DMA-BUF for
//!   the rest of the session: the stream offers shared memory only and
//!   PipeWire negotiates the format again (`fall_back`); no new portal
//!   dialog, the stream and its node stay.
//! - Lifetime: a held picture keeps its PipeWire buffer dequeued and its own
//!   duplicate of the descriptor; the buffer goes back to PipeWire when the
//!   holder drops it, which the encoder does right after the video processor
//!   finished reading it (`vaSyncSurface`). Only the newest picture is held;
//!   an older one goes back when a newer arrives (Sunshine `on_process`).
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::super::linux_hw_codecs::{DmaBufFrame, HeldDmaBuf, ImportTarget};
use super::super::spa_pod::dmabuf::{offer_pods, Damage, DmaBufOffer, Negotiated};
use super::super::spa_pod::VideoFormat;
use crate::screen::core::capture_admission::{CaptureGeneration, SourceKey};

/// Whether the host pipeline hands DMA-BUF pictures to the encoder. On in
/// the host process (merge W2: `os::enable_gpu_pictures` at its start, the
/// only process whose picture stream carries `capture_picture::Picture::Gpu`
/// to `ScreenEncoder::encode_picture`); while off the stream offers shared
/// memory exactly as before.
static GPU_PIPELINE: AtomicBool = AtomicBool::new(false);

/// Switches the DMA-BUF offer on for streams started afterwards.
pub(in crate::screen::os) fn enable_gpu_pipeline(on: bool) {
    GPU_PIPELINE.store(on, Ordering::Release);
}

pub(super) fn gpu_pipeline_ready() -> bool {
    GPU_PIPELINE.load(Ordering::Acquire)
}

/// The memory a stream asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BufferKind {
    DmaBuf,
    Shm,
}

/// Why a stream left DMA-BUF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fallback {
    /// The producer chose a modifier or format this host did not offer.
    Modifier,
    /// The buffer's planes cannot be described to the import (several
    /// objects, a plane of another type, more than four planes).
    Layout,
    /// `vaCreateSurfaces` refused the buffer.
    Import,
    /// The video processor failed on it.
    Conversion,
    /// The session's encoder takes CPU pictures (software, tiles, NVENC).
    Consumer,
    /// The viewer draws no pointer, the host mixes it into the picture.
    PointerBlend,
}

impl From<super::super::linux_hw_codecs::DmaBufCause> for Fallback {
    fn from(cause: super::super::linux_hw_codecs::DmaBufCause) -> Self {
        use super::super::linux_hw_codecs::DmaBufCause;
        match cause {
            DmaBufCause::Import => Self::Import,
            DmaBufCause::Conversion => Self::Conversion,
            DmaBufCause::Consumer => Self::Consumer,
        }
    }
}

impl Fallback {
    pub(super) fn text(self) -> &'static str {
        match self {
            Self::Modifier => "dmabuf_modifier",
            Self::Layout => "dmabuf_layout",
            Self::Import => "dmabuf_import",
            Self::Conversion => "dmabuf_conversion",
            Self::Consumer => "dmabuf_consumer_cpu",
            Self::PointerBlend => "dmabuf_pointer_blend",
        }
    }
}

/// The offers of an import target (formats the stream knows, with
/// modifiers).
pub(super) fn offers_for(target: &ImportTarget) -> Vec<DmaBufOffer> {
    target
        .formats
        .iter()
        .filter(|(_, modifiers)| !modifiers.is_empty())
        .filter_map(|(drm, modifiers)| {
            Some(DmaBufOffer {
                format: VideoFormat::from_drm_fourcc(*drm)?,
                modifiers: modifiers.clone(),
            })
        })
        .collect()
}

/// B-15: DMA-BUF when the pipeline passes GPU pictures, the GPU encoder
/// imports at least one layout, it is the session's encoder and the host
/// need not mix the pointer into the picture; else shared memory.
pub(super) fn choose(
    pipeline_ready: bool,
    offers: &[DmaBufOffer],
    encoder_imports: bool,
    pointer_blends: bool,
) -> BufferKind {
    if pipeline_ready && !offers.is_empty() && encoder_imports && !pointer_blends {
        BufferKind::DmaBuf
    } else {
        BufferKind::Shm
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Shared memory only; the cause when the stream left DMA-BUF.
    Shm(Option<Fallback>),
    /// DMA-BUF offered, no format yet or the producer chose shared memory.
    Offered,
    /// DMA-BUF negotiated with this format and modifier.
    DmaBuf(VideoFormat, u64),
}

struct Inner {
    offers: Vec<DmaBufOffer>,
    node: Option<PathBuf>,
    state: State,
    /// The producer filled damage metadata at least once with a region.
    damage_seen: bool,
}

/// The DMA-BUF negotiation of one stream.
pub(super) struct DmaBufSession {
    inner: Mutex<Inner>,
}

impl Default for DmaBufSession {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner {
                offers: Vec::new(),
                node: None,
                state: State::Shm(None),
                damage_seen: false,
            }),
        }
    }
}

impl DmaBufSession {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The `EnumFormat` pods of the connection. `target` is asked only when
    /// the pipeline passes GPU pictures. The encoder of the session is not
    /// open yet; the target names the encoder that ranks first for it, and
    /// `fall_back(Consumer)` corrects a different choice.
    pub(super) fn connect_pods(&self, target: impl FnOnce() -> ImportTarget) -> Vec<Vec<u8>> {
        let mut inner = self.lock();
        if !gpu_pipeline_ready() || matches!(inner.state, State::Shm(Some(_))) {
            return offer_pods(&[]);
        }
        let target = target();
        let offers = offers_for(&target);
        if choose(true, &offers, target.node.is_some(), false) == BufferKind::Shm {
            return offer_pods(&[]);
        }
        inner.state = State::Offered;
        inner.node = target.node;
        inner.offers = offers;
        offer_pods(&inner.offers)
    }

    /// The negotiated format decides the memory of the buffers. `Err` when
    /// the producer chose a modifier or format this host did not offer.
    pub(super) fn on_format(&self, negotiated: &Negotiated) -> Result<BufferKind, Fallback> {
        let mut inner = self.lock();
        let Some(modifier) = negotiated.modifier else {
            // The producer took the shared memory offer; DMA-BUF stays offered.
            if !matches!(inner.state, State::Shm(_)) {
                inner.state = State::Offered;
            }
            return Ok(BufferKind::Shm);
        };
        if matches!(inner.state, State::Shm(_)) {
            return Err(Fallback::Modifier);
        }
        // A format whose modifier the producer has not fixated yet carries
        // the list; only its format is checked now, the fixated one follows.
        let offered = inner.offers.iter().any(|offer| {
            offer.format == negotiated.format
                && (negotiated.fixation_pending || offer.modifiers.contains(&modifier))
        });
        if !offered {
            return Err(Fallback::Modifier);
        }
        inner.state = State::DmaBuf(negotiated.format, modifier);
        Ok(BufferKind::DmaBuf)
    }

    /// Leaves DMA-BUF for the rest of the session: the shared memory offer
    /// to renegotiate with, `None` when the stream already is on shared
    /// memory for good (nothing to renegotiate).
    pub(super) fn fall_back(&self, cause: Fallback) -> Option<Vec<Vec<u8>>> {
        let mut inner = self.lock();
        match inner.state {
            State::Shm(Some(_)) => None,
            State::Shm(None) => {
                inner.state = State::Shm(Some(cause));
                None
            }
            State::Offered | State::DmaBuf(..) => {
                inner.state = State::Shm(Some(cause));
                inner.offers.clear();
                Some(offer_pods(&[]))
            }
        }
    }

    /// The negotiated DMA-BUF format and modifier, if any.
    pub(super) fn dmabuf(&self) -> Option<(VideoFormat, u64)> {
        match self.lock().state {
            State::DmaBuf(format, modifier) => Some((format, modifier)),
            _ => None,
        }
    }

    /// Why the stream left DMA-BUF (for the session's statistics; the
    /// process-wide report is `capture_route::report`).
    #[allow(dead_code)] // Read by task tests; the report carries the cause.
    pub(super) fn fallback(&self) -> Option<Fallback> {
        match self.lock().state {
            State::Shm(cause) => cause,
            _ => None,
        }
    }

    /// The render node the target probed (the still image reads back there).
    pub(super) fn node(&self) -> Option<PathBuf> {
        self.lock().node.clone()
    }

    /// Whether a buffer counts as a new picture: without damage metadata
    /// every buffer does (compositors send a buffer only for a change or a
    /// pointer move, and a pointer-only buffer has an empty chunk); with it,
    /// a buffer whose first region is invalid is no change, but only once
    /// the producer has shown that it fills the block (a block it never
    /// writes reads as all zero, which would else hide every picture).
    pub(super) fn changed(&self, damage: Option<Damage>) -> bool {
        let mut inner = self.lock();
        match damage {
            None => true,
            Some(Damage::Regions(count)) => {
                inner.damage_seen |= count > 0;
                true
            }
            Some(Damage::Nothing) => !inner.damage_seen,
        }
    }

    /// A new format starts the damage evidence over.
    pub(super) fn reset_damage(&self) {
        self.lock().damage_seen = false;
    }
}

/// Gives a held buffer back to its stream (implemented on the stream).
pub(super) trait BufferReturn: Send + Sync {
    fn give_back(&self, buffer: usize);
    fn give_back_token(&self, buffer: usize, _token: u64) {
        self.give_back(buffer);
    }
}

/// One DMA-BUF picture held for the encoder.
pub(super) struct HeldBuffer {
    buffer: usize,
    token: u64,
    frame: DmaBufFrame,
    fd: Arc<OwnedFd>,
    source: SourceKey,
    generation: CaptureGeneration,
    returner: Option<Arc<dyn BufferReturn>>,
}

impl HeldBuffer {
    /// `fd` is this program's duplicate of the plane descriptor; `frame.fd`
    /// is set to it.
    pub(super) fn new(
        buffer: usize,
        mut frame: DmaBufFrame,
        fd: OwnedFd,
        source: SourceKey,
        generation: CaptureGeneration,
        returner: Arc<dyn BufferReturn>,
    ) -> Self {
        frame.fd = fd.as_raw_fd();
        Self {
            buffer,
            token: 0,
            frame,
            fd: Arc::new(fd),
            source,
            generation,
            returner: Some(returner),
        }
    }

    pub(super) fn with_token(mut self, token: u64) -> Self {
        self.token = token;
        self
    }

    /// The picture and the descriptor it lives on, for a read-back after
    /// the buffer went back (merge W2: `DmaBufPicture`).
    pub(super) fn still(&self) -> Still {
        (self.frame.clone(), self.fd.clone())
    }

    /// The buffer without giving it back: for the stream's own thread, which
    /// queues it itself under the loop lock it already holds.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn disarm(mut self) -> usize {
        self.returner = None;
        self.buffer
    }

    pub(super) fn disarm_token(mut self) -> (usize, u64) {
        self.returner = None;
        (self.buffer, self.token)
    }
}

impl Drop for HeldBuffer {
    fn drop(&mut self) {
        if let Some(returner) = self.returner.take() {
            returner.give_back_token(self.buffer, self.token);
        }
    }
}

impl HeldDmaBuf for HeldBuffer {
    fn frame(&self) -> &DmaBufFrame {
        &self.frame
    }
    fn identity(&self) -> (&SourceKey, CaptureGeneration) {
        (&self.source, self.generation)
    }
}

/// What `GpuSlot::take` found.
pub(super) enum GpuTaken {
    Frame(HeldBuffer),
    /// A key frame without a new picture: the encoder's input still holds
    /// the last converted picture (`HwH264Encoder::encode_dmabuf(None)`).
    Repeat,
    Unchanged,
}

/// A picture for a still image: the frame and the descriptor it lives on.
pub(super) type Still = (DmaBufFrame, Arc<OwnedFd>);

#[derive(Default)]
struct SlotInner {
    newest: Option<HeldBuffer>,
    /// The last picture handed to the encoder, kept only as its descriptor
    /// (its PipeWire buffer went back) for a still image of a screen that
    /// did not change since.
    last: Option<Still>,
}

/// The newest DMA-BUF picture of a stream.
#[derive(Default)]
pub(super) struct GpuSlot {
    inner: Mutex<SlotInner>,
}

impl GpuSlot {
    fn lock(&self) -> MutexGuard<'_, SlotInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps `held`; returns the older picture it replaces, which the
    /// caller gives back outside this slot's lock.
    pub(super) fn publish(&self, held: HeldBuffer) -> Option<HeldBuffer> {
        self.lock().newest.replace(held)
    }

    /// The newest picture once; with `keyframe` and none new a repeat of
    /// the last delivered one.
    pub(super) fn take(&self, keyframe: bool) -> GpuTaken {
        let mut inner = self.lock();
        match inner.newest.take() {
            Some(held) => {
                inner.last = Some((held.frame.clone(), held.fd.clone()));
                GpuTaken::Frame(held)
            }
            None if keyframe && inner.last.is_some() => GpuTaken::Repeat,
            None => GpuTaken::Unchanged,
        }
    }

    /// The picture a still image reads back: the newest held one (its
    /// buffer stays held, so its content is stable), else the last one the
    /// encoder took (its buffer went back; the compositor renders into it
    /// again only for a newer picture, which then is the newest here).
    pub(super) fn still(&self) -> Option<Still> {
        let inner = self.lock();
        match &inner.newest {
            Some(held) => Some((held.frame.clone(), held.fd.clone())),
            None => inner.last.clone(),
        }
    }

    /// Empties the slot (pause, format change, end); the caller gives the
    /// picture back. The next key frame needs a new picture.
    pub(super) fn clear(&self) -> Option<HeldBuffer> {
        let mut inner = self.lock();
        inner.last = None;
        inner.newest.take()
    }
}

#[cfg(test)]
#[path = "pipewire_dmabuf_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "bildkette_w2_dmabuf_task_tests.rs"]
mod w2_tests;
