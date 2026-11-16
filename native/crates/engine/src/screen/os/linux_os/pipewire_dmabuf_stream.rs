//! The PipeWire side of the DMA-BUF capture (plan 2026-11-14, BK8b): the
//! offer at connection, the answer to a negotiated format, the held picture
//! of a dequeued DMA-BUF buffer, the renegotiation to shared memory and the
//! one read-back of a still image. Every function named `*_locked` or
//! called from a stream callback runs on the loop's thread with its lock.
//! Merge W2: the newest held picture leaves as the GPU picture of the host
//! pipeline (`take_gpu_picture`), whose consumers end DMA-BUF through
//! `StreamFallback`; the route and its fallback cause are reported
//! (`core::capture_route`).
use super::super::linux_hw_codecs::{
    dmabuf_import_target, read_bgrx, DmaBufCause, DmaBufFrame, DmaBufPicture, DmaBufStream,
    HeldDmaBuf,
};
use super::super::pipewire_abi::memory::fd_facts;
use super::super::pipewire_abi::SpaData;
use super::super::spa_pod::dmabuf::{
    damage_meta_pod, dmabuf_buffers_pod, parse_damage, Damage, Negotiated, DATA_DMABUF,
    META_VIDEO_DAMAGE,
};
use super::dmabuf::{BufferKind, Fallback, GpuTaken, HeldBuffer};
use super::*;
use crate::screen::core::capture_admission::CaptureOwner;
use crate::screen::core::capture_picture::GpuPicture;
use crate::screen::core::capture_route::{self, CaptureRoute};
use std::os::fd::FromRawFd;
use std::sync::atomic::AtomicU64;

/// The `EnumFormat` pods the stream connects with.
pub(super) fn connect_pods(latest: &LatestFrame) -> Vec<Vec<u8>> {
    latest.dmabuf.connect_pods(dmabuf_import_target)
}

/// `pw_stream_update_params` with `pods`.
///
/// # Safety
/// The loop lock is held and `stream` is the live stream (or null).
pub(super) unsafe fn update_formats(
    api: &PipewireApi,
    stream: *mut c_void,
    pods: &[Vec<u8>],
) -> io::Result<()> {
    if stream.is_null() {
        return Err(io::Error::other("pipewire stream closed"));
    }
    let words: Vec<Vec<u64>> = pods.iter().map(|pod| pod_words(pod)).collect();
    let mut params: Vec<*const SpaPod> = words.iter().map(|w| w.as_ptr().cast()).collect();
    let result = (api.stream_update_params)(stream, params.as_mut_ptr(), params.len() as u32);
    if result < 0 {
        return Err(io::Error::other(format!(
            "pipewire format renegotiation failed: {result}"
        )));
    }
    Ok(())
}

/// One line on the host process's error output per fallback (as BK8a's
/// `cursor_embedded`).
pub(super) fn report(cause: Fallback) {
    capture_route::note_fallback(cause.text());
    eprintln!(
        "screen host process: pipewire: {}: shared memory for this session",
        cause.text()
    );
}

/// Leaves DMA-BUF for the rest of the session from the loop's thread: the
/// held picture goes back and PipeWire negotiates the shared memory offer.
///
/// # Safety
/// On the loop's thread with its lock.
pub(super) unsafe fn fall_back_locked(data: &CallbackData, cause: Fallback) {
    let Some(pods) = data.latest.dmabuf.fall_back(cause) else {
        return;
    };
    data.latest.clear_gpu_locked();
    let stream = data.stream.load(Ordering::Acquire);
    if let Err(error) = update_formats(&data.api, stream, &pods) {
        data.fail(error.to_string());
        return;
    }
    report(cause);
}

/// The parameters answering a negotiated format: the `Buffers` request for
/// its memory and the metadata requests; `None` after a fallback (the next
/// format comes from the renegotiation).
///
/// # Safety
/// From `param_changed` (loop thread, lock held).
pub(super) unsafe fn format_params(
    data: &CallbackData,
    negotiated: &Negotiated,
) -> io::Result<Option<Vec<Vec<u8>>>> {
    data.latest.dmabuf.reset_damage();
    let kind = data.latest.dmabuf.on_format(negotiated);
    if let Ok(kind) = kind {
        capture_route::note_route(match kind {
            BufferKind::DmaBuf => CaptureRoute::DmaBuf,
            BufferKind::Shm => CaptureRoute::SharedMemory,
        });
    }
    match kind {
        Ok(BufferKind::DmaBuf) => Ok(Some(vec![
            dmabuf_buffers_pod(),
            super::cursor::request_pod(),
            damage_meta_pod(),
        ])),
        // Functional size request only: native imports are separately
        // admitted at Core.add_mem before their first local mapping.
        Ok(BufferKind::Shm) => Ok(Some(vec![
            super::super::spa_pod::consumer_buffers_pod(negotiated.size)?,
            super::cursor::request_pod(),
        ])),
        Err(cause) => {
            fall_back_locked(data, cause);
            Ok(None)
        }
    }
}

/// The damage block of `spa`, if the producer attached one.
unsafe fn damage(spa: &super::super::pipewire_abi::SpaBuffer) -> Option<Damage> {
    if spa.n_metas == 0 || spa.metas.is_null() {
        return None;
    }
    std::slice::from_raw_parts(spa.metas, spa.n_metas as usize)
        .iter()
        .find(|meta| meta.type_ == META_VIDEO_DAMAGE && meta.size != 0 && !meta.data.is_null())
        .map(|meta| {
            parse_damage(std::slice::from_raw_parts(
                meta.data.cast::<u8>(),
                meta.size as usize,
            ))
        })
}

/// The planes of a DMA-BUF buffer as `(offset, pitch)`: every data block a
/// DMA-BUF on the same descriptor (one object), at most four.
pub(super) fn planes(datas: &[(u32, i64, u32, i32)]) -> Result<Vec<(u32, u32)>, Fallback> {
    let first = datas.first().ok_or(Fallback::Layout)?.1;
    if datas.len() > 4 {
        return Err(Fallback::Layout);
    }
    datas
        .iter()
        .map(|(kind, fd, offset, stride)| {
            if *kind != DATA_DMABUF || *fd != first || *stride <= 0 {
                return Err(Fallback::Layout);
            }
            Ok((*offset, *stride as u32))
        })
        .collect()
}

/// The held picture of a dequeued buffer; `Ok(None)` when the buffer has no
/// new picture or the capture may not read it now.
///
/// # Safety
/// On the loop's thread with a dequeued `buffer` of this stream.
unsafe fn held(data: &CallbackData, buffer: *mut PwBuffer) -> Result<Option<HeldBuffer>, Fallback> {
    let Some((format, modifier)) = data.latest.dmabuf.dmabuf() else {
        return Ok(None);
    };
    let (Some((_, size)), Some(grant)) = (*lock_format(data), data.memory.current_grant()) else {
        return Ok(None);
    };
    let known = grant
        .check_identity(data.memory.source(), data.memory.generation())
        .is_ok()
        && data
            .owners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(buffer, grant.generation());
    if data.latest.import_blocked() || !known {
        return Ok(None);
    }
    // The same admission checks as the shared memory copy: every chunk and
    // metadata range lies in an admitted import, every plane descriptor is an
    // admitted DMA-BUF import (`dmabuf_owner`).
    if let Err(error) = data.imports.validate_buffer(buffer) {
        data.fail(error.to_string());
        return Ok(None);
    }
    let spa = &*(*buffer).buffer;
    let datas: &[SpaData] = std::slice::from_raw_parts(spa.datas, spa.n_datas as usize);
    let chunk = &*datas[0].chunk;
    // A pointer-only or corrupted buffer has no picture (Sunshine `fill_img`).
    if chunk.flags & SPA_CHUNK_FLAG_CORRUPTED != 0 || chunk.size == 0 {
        data.latest.trace.empty_chunk();
        return Ok(None);
    }
    if !data.latest.dmabuf.changed(damage(spa)) {
        return Ok(None);
    }
    let drm_format = format.drm_fourcc().ok_or(Fallback::Modifier)?;
    let fd = c_int::try_from(datas[0].fd).map_err(|_| Fallback::Layout)?;
    let first_facts = fd_facts(fd).map_err(|_| Fallback::Import)?;
    let described: Vec<(u32, i64, u32, i32)> = datas
        .iter()
        .enumerate()
        .map(|(index, block)| {
            let chunk = &*block.chunk;
            let same_object = index > 0
                && block.type_ == DATA_DMABUF
                && block.fd != datas[0].fd
                && c_int::try_from(block.fd)
                    .ok()
                    .and_then(|other| fd_facts(other).ok())
                    == Some(first_facts);
            let fd_key = if same_object { datas[0].fd } else { block.fd };
            (block.type_, fd_key, chunk.offset, chunk.stride)
        })
        .collect();
    let planes = planes(&described)?;
    let object_size = first_facts.length;
    if planes.iter().any(|(offset, _)| *offset >= object_size) {
        return Err(Fallback::Layout);
    }
    let token = data.latest.gpu_token(buffer).ok_or(Fallback::Layout)?;
    // Own duplicate: valid until the import surface is gone, whatever the
    // stream does with its descriptor (va_drmcommon.h: the driver does not
    // close it, closing does not end the surface).
    let duplicate = libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0);
    if duplicate < 0 {
        return Err(Fallback::Import);
    }
    let owned = OwnedFd::from_raw_fd(duplicate);
    let frame = DmaBufFrame {
        fd: duplicate,
        drm_format,
        modifier,
        size,
        planes,
        object_size,
    };
    Ok(Some(
        HeldBuffer::new(
            buffer as usize,
            frame,
            owned,
            data.memory.source().clone(),
            grant.generation(),
            data.returner.clone(),
        )
        .with_token(token),
    ))
}

/// One dequeued buffer of a DMA-BUF stream: `true` when the stream keeps it
/// (the newest held picture), `false` when the caller queues it back now.
///
/// # Safety
/// On the loop's thread with a dequeued `buffer` of this stream.
pub(super) unsafe fn process(data: &CallbackData, buffer: *mut PwBuffer) -> bool {
    // Pointer metadata keeps reaching the overlay; a pointer the host has to
    // mix in needs the CPU picture, so the session returns to shared memory.
    let _ = data.cursor.process(data, buffer, None);
    if data.cursor.blends() {
        fall_back_locked(data, Fallback::PointerBlend);
        return false;
    }
    match held(data, buffer) {
        Ok(Some(held)) => match data.latest.publish_gpu(held) {
            Ok(()) => true,
            Err(held) => {
                let _ = held.disarm();
                false
            }
        },
        Ok(None) => false,
        Err(cause) => {
            fall_back_locked(data, cause);
            false
        }
    }
}

/// What the GPU route of a capture found (merge W2).
pub(in crate::screen::os) enum GpuCapture {
    Picture(GpuPicture),
    Unchanged,
}

/// The stream a GPU picture leaves DMA-BUF through (`DmaBufStream`); weak,
/// so a picture waiting in the host pipeline does not keep the stream.
pub(super) struct StreamFallback(pub(super) std::sync::Weak<LatestFrame>);

impl DmaBufStream for StreamFallback {
    fn fall_back(&self, cause: DmaBufCause) {
        if let Some(latest) = self.0.upgrade() {
            if let Err(error) = latest.fall_back_from_worker(Fallback::from(cause)) {
                latest.fail(error.to_string());
            }
        }
    }
}

/// Diagnosis number of the GPU pictures of this process (`GpuSurface::slot`).
static GPU_PICTURES: AtomicU64 = AtomicU64::new(0);

impl LatestFrame {
    /// A still image of a DMA-BUF stream (BK8b result 5): the newest picture
    /// read back once through VA into admitted capture pixels (owner
    /// `SourcePixels`, as the shared memory copy), exactly the BGRx of the
    /// source with alpha set as there (`linux_hw_codecs::read_bgrx`).
    /// `Ok(None)` while there is no picture. The source's `capture_snapshot`
    /// calls this while `dmabuf.dmabuf()` is `Some` (merge W2).
    pub(in crate::screen::os) fn still_from_gpu(
        &self,
        grant: &CaptureGrant,
    ) -> io::Result<Option<CapturedImage>> {
        let (Some(_), Some(node), Some((frame, _descriptor))) =
            (self.dmabuf.dmabuf(), self.dmabuf.node(), self.gpu.still())
        else {
            return Ok(None);
        };
        read_bgrx(&node, &frame, grant, CaptureOwner::SourcePixels).map(Some)
    }

    /// While DMA-BUF is negotiated: the newest picture as a GPU picture of
    /// `grant` (BK8b hand-off Ü1c, merge W2), a repeat of the last one for a
    /// key frame, or `Unchanged`; `Ok(None)` while the stream is on shared
    /// memory (the caller takes the shared memory picture).
    pub(in crate::screen::os) fn take_gpu_picture(
        self: &Arc<Self>,
        keyframe: bool,
        grant: &CaptureGrant,
    ) -> io::Result<Option<GpuCapture>> {
        let (Some(_), Some(node)) = (self.dmabuf.dmabuf(), self.dmabuf.node()) else {
            return Ok(None);
        };
        let (held, still) = match self.take_gpu(keyframe)? {
            GpuTaken::Frame(held) => {
                let still = held.still();
                (Some(Box::new(held) as Box<dyn HeldDmaBuf + Send>), still)
            }
            GpuTaken::Repeat => match self.gpu.still() {
                Some(still) => (None, still),
                None => return Ok(Some(GpuCapture::Unchanged)),
            },
            GpuTaken::Unchanged => return Ok(Some(GpuCapture::Unchanged)),
        };
        let size = still.0.size;
        let stream: Arc<dyn DmaBufStream> = Arc::new(StreamFallback(Arc::downgrade(self)));
        let slot = GPU_PICTURES.fetch_add(1, Ordering::Relaxed);
        let surface = DmaBufPicture::new(held, still, node, stream, slot);
        let picture = GpuPicture::new(Arc::new(surface), size, FrameLimits::RESOURCE, grant)?;
        Ok(Some(GpuCapture::Picture(picture)))
    }
}
