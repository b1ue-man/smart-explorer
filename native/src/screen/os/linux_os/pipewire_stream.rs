//! The PipeWire video stream of a portal screen cast (SCR-M5, U19-5): joins
//! the portal's PipeWire remote with its file descriptor, connects an input
//! stream to the portal's node, negotiates raw video as shared memory
//! (`spa_pod::enum_format_pod`) and copies every picture into `LatestFrame`.
//! All calls on the stream and the core run between `lock` and `unlock` of a
//! `pw_thread_loop`; the callbacks run on the loop's thread. `capture` of
//! the source takes the picture from `LatestFrame` on another thread.
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io;
use std::os::fd::{IntoRawFd, OwnedFd};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::pipewire_abi::{
    PipewireApi, PwBuffer, PwStreamEvents, SpaHook, SpaPod, PW_DIRECTION_INPUT,
    PW_STREAM_FLAG_AUTOCONNECT, PW_STREAM_FLAG_MAP_BUFFERS, PW_STREAM_STATE_ERROR,
    PW_VERSION_STREAM_EVENTS, SPA_CHUNK_FLAG_CORRUPTED, SPA_PARAM_FORMAT,
};
use super::spa_pod::{enum_format_pod, parse_video_format, VideoFormat};
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};

#[path = "pipewire_trace.rs"]
mod trace;
use trace::Trace;
#[path = "pipewire_identity.rs"]
pub(super) mod identity;
use identity::StreamIdentity;

/// What `take` found.
pub(crate) enum Taken {
    Frame(BgraImage),
    Unchanged,
    /// The stream failed; the text names the cause.
    Failed(String),
}

#[path = "pipewire_pause.rs"]
mod pause;
pub(crate) use pause::LatestFrame;

/// Copies `size.height` rows of `src` (rows `stride` bytes apart, 4 bytes
/// per pixel in `format`) into a tight BGRA image with opaque alpha.
pub(crate) fn convert_rows(
    src: &[u8],
    stride: usize,
    format: VideoFormat,
    size: FrameSize,
) -> io::Result<BgraImage> {
    let (width, height) = (size.width as usize, size.height as usize);
    let row_bytes = width * 4;
    let needed = stride
        .checked_mul(height.saturating_sub(1))
        .and_then(|bytes| bytes.checked_add(row_bytes))
        .ok_or_else(|| io::Error::other("pipewire_buffer: size overflow"))?;
    if stride < row_bytes || src.len() < needed {
        return Err(io::Error::other(
            "pipewire_buffer: the buffer is smaller than its picture",
        ));
    }
    let mut image = BgraImage::new(size)?;
    let swap = format.swaps_red_blue();
    for (y, row) in src.chunks(stride).take(height).enumerate() {
        let target = image.row_mut(y as u32);
        target.copy_from_slice(&row[..row_bytes]);
        for pixel in target.chunks_exact_mut(4) {
            if swap {
                pixel.swap(0, 2);
            }
            pixel[3] = 0xFF;
        }
    }
    Ok(image)
}

/// What the callbacks need; boxed, so its address stays.
struct CallbackData {
    api: Arc<PipewireApi>,
    latest: Arc<LatestFrame>,
    stream: AtomicPtr<c_void>,
    format: Mutex<Option<(VideoFormat, FrameSize)>>,
    identity: Arc<StreamIdentity>,
}

impl CallbackData {
    fn fail(&self, reason: String) {
        self.latest.fail(reason);
        self.identity.invalidate();
    }
}

fn guarded(data: *mut c_void, work: impl FnOnce(&CallbackData)) {
    // SAFETY: `data` is the `CallbackData` box that `PwStream` keeps alive
    // until the stream is destroyed.
    let data = unsafe { &*(data as *const CallbackData) };
    let outcome = catch_unwind(AssertUnwindSafe(|| work(data)));
    if outcome.is_err() {
        data.fail("pipewire_callback: panic".into());
    }
}

unsafe extern "C" fn on_state_changed(
    data: *mut c_void,
    old: c_int,
    new: c_int,
    error: *const c_char,
) {
    guarded(data, |data| {
        data.latest.trace.state(new);
        data.latest.on_state_changed(old,new);
        data.identity.on_state(old, new);
        if new == PW_STREAM_STATE_ERROR {
            let text = if error.is_null() {
                "unknown".to_string()
            } else {
                // SAFETY: PipeWire passes a NUL-terminated string.
                unsafe { CStr::from_ptr(error) }
                    .to_string_lossy()
                    .into_owned()
            };
            data.fail(format!("pipewire_stream: {text}"));
        } else if data.identity.expired() {
            data.latest.fail("portal_source_expired: choose the source again".into());
        }
    });
}

unsafe extern "C" fn on_param_changed(data: *mut c_void, id: u32, param: *const SpaPod) {
    guarded(data, |data| {
        if id != SPA_PARAM_FORMAT {
            return;
        }
        let negotiated = if param.is_null() {
            None
        } else {
            // SAFETY: a pod is its 8-byte header and `size` bytes of body.
            let bytes = unsafe {
                let size = (*param).size as usize;
                std::slice::from_raw_parts(param.cast::<u8>(), 8 + size)
            };
            parse_video_format(bytes)
        };
        if param.is_null() {
            data.latest.trace.format("none");
            *lock_format(data) = None;
            return;
        }
        match negotiated {
            Some((format, size)) if size.is_valid_frame() => {
                data.latest
                    .trace
                    .format(&format!("{format:?} {}x{}", size.width, size.height));
                *lock_format(data) = Some((format, size));
            }
            Some((_, size)) => data.fail(format!(
                "pipewire_size: {}x{} is outside the 4096 x 2304 frame limit",
                size.width, size.height
            )),
            None => data.fail("pipewire_format".into()),
        }
    });
}

fn lock_format(data: &CallbackData) -> MutexGuard<'_, Option<(VideoFormat, FrameSize)>> {
    data.format.lock().unwrap_or_else(PoisonError::into_inner)
}

unsafe extern "C" fn on_process(data: *mut c_void) {
    guarded(data, |data| {
        let stream = data.stream.load(Ordering::Acquire);
        data.latest.trace.process();
        let mut last: *mut PwBuffer = null_mut();
        // SAFETY: called on the loop's thread with the stream alive; each
        // dequeued buffer goes back with `queue_buffer` exactly once.
        unsafe {
            loop {
                let next = (data.api.stream_dequeue_buffer)(stream);
                if next.is_null() {
                    break;
                }
                if !last.is_null() {
                    (data.api.stream_queue_buffer)(stream, last);
                }
                last = next;
            }
            if last.is_null() {
                return;
            }
            if data.latest.is_capture_active() && data.identity.connected() {
                if let Some(image) = copy_picture(data, last) { data.latest.publish(image); }
            }
            (data.api.stream_queue_buffer)(stream, last);
        }
    });
}

/// The picture of `buffer`, or `None` for a buffer without content (the
/// header buffer of a stream that only moved its cursor) or while no format
/// is negotiated.
///
/// # Safety
/// `buffer` is a buffer just dequeued from the stream.
unsafe fn copy_picture(data: &CallbackData, buffer: *mut PwBuffer) -> Option<BgraImage> {
    let skip = |reason: &str| data.latest.trace.skip(reason);
    let Some((format, size)) = *lock_format(data) else {
        skip("no format negotiated yet");
        return None;
    };
    let spa = (*buffer).buffer;
    if spa.is_null() || (*spa).n_datas == 0 || (*spa).datas.is_null() {
        skip("buffer without data blocks");
        return None;
    }
    let block = &*(*spa).datas;
    if block.data.is_null() || block.chunk.is_null() {
        // 3 is `SPA_DATA_DmaBuf`: not mapped, only a descriptor.
        skip(&format!("block of type {} is not mapped", block.type_));
        return None;
    }
    let chunk = &*block.chunk;
    // A corrupted chunk ("do not look at the data") or one of size 0 carries
    // no picture: KWin sends both kinds, a size-0 chunk flagged corrupted, for
    // a move of the cursor in a stream with cursor metadata.
    if chunk.flags & SPA_CHUNK_FLAG_CORRUPTED != 0 || chunk.size == 0 {
        data.latest.trace.empty_chunk();
        skip("chunk without picture (size 0 or corrupted: a buffer without damage)");
        return None;
    }
    let row = size.width as usize * 4;
    let stride = if chunk.stride > 0 {
        chunk.stride as usize
    } else {
        row
    };
    let offset = chunk.offset as usize;
    let mapped = block.maxsize as usize;
    if offset >= mapped {
        data.fail("pipewire_buffer: offset beyond the buffer".into());
        return None;
    }
    let bytes = std::slice::from_raw_parts(block.data.cast::<u8>().add(offset), mapped - offset);
    match convert_rows(bytes, stride, format, size) {
        Ok(image) => Some(image),
        Err(error) => {
            data.fail(error.to_string());
            None
        }
    }
}

/// A running stream; dropping it stops and destroys everything.
pub(crate) struct PwStream {
    api: Arc<PipewireApi>,
    thread_loop: *mut c_void,
    context: *mut c_void,
    core: *mut c_void,
    stream: *mut c_void,
    // Kept for the callbacks until the stream is destroyed.
    _hook: Box<SpaHook>,
    _events: Box<PwStreamEvents>,
    _data: Box<CallbackData>,
}

// SAFETY: every PipeWire object is touched only between `lock` and `unlock`
// of the thread loop, or after the loop is stopped (drop).
unsafe impl Send for PwStream {}

impl PwStream {
    /// Joins the PipeWire remote behind `fd` and connects to `node_id`; the
    /// pictures arrive in `latest`.
    pub(crate) fn start(
        api: Arc<PipewireApi>,
        fd: OwnedFd,
        identity: Arc<StreamIdentity>,
        latest: Arc<LatestFrame>,
    ) -> io::Result<Self> {
        let name = CString::new("smart-explorer-screen").map_err(io::Error::other)?;
        identity.attach(Arc::downgrade(&latest));
        if identity.expired() { return Err(io::Error::other("portal_source_expired")); }
        latest.suspend_before_connect();
        let mut data = Box::new(CallbackData {
            api: api.clone(),
            latest,
            stream: AtomicPtr::new(null_mut()),
            format: Mutex::new(None),
            identity,
        });
        let mut events = Box::new(PwStreamEvents {
            version: PW_VERSION_STREAM_EVENTS,
            destroy: None,
            state_changed: Some(on_state_changed),
            control_info: None,
            io_changed: None,
            param_changed: Some(on_param_changed),
            add_buffer: None,
            remove_buffer: None,
            process: Some(on_process),
            drained: None,
            command: None,
            trigger_done: None,
        });
        let mut hook = Box::new(SpaHook::zeroed());
        // SAFETY: the calls follow the documented PipeWire order; every
        // object is checked for null and given back on failure.
        let created =
            unsafe { create(&api, fd, &name, &mut data, &mut events, &mut hook) };
        let (thread_loop, context, core, stream) = created?;
        data.latest.attach(api.clone(),thread_loop,stream);
        Ok(Self {
            api,
            thread_loop,
            context,
            core,
            stream,
            _hook: hook,
            _events: events,
            _data: data,
        })
    }
}

/// Builds the loop, context, core and stream and connects the stream.
///
/// # Safety
/// `data`, `events` and `hook` must stay at their addresses until the stream
/// is destroyed.
unsafe fn create(
    api: &Arc<PipewireApi>,
    fd: OwnedFd,
    name: &CString,
    data: &mut CallbackData,
    events: &mut PwStreamEvents,
    hook: &mut SpaHook,
) -> io::Result<(*mut c_void, *mut c_void, *mut c_void, *mut c_void)> {
    let fail = |what: &str| io::Error::other(format!("pipewire: {what} failed"));
    let thread_loop = (api.thread_loop_new)(name.as_ptr(), null());
    if thread_loop.is_null() {
        return Err(fail("pw_thread_loop_new"));
    }
    let context = (api.context_new)((api.thread_loop_get_loop)(thread_loop), null_mut(), 0);
    if context.is_null() {
        (api.thread_loop_destroy)(thread_loop);
        return Err(fail("pw_context_new"));
    }
    if (api.thread_loop_start)(thread_loop) < 0 {
        (api.context_destroy)(context);
        (api.thread_loop_destroy)(thread_loop);
        return Err(fail("pw_thread_loop_start"));
    }
    (api.thread_loop_lock)(thread_loop);
    // The core owns the descriptor from here on.
    let core = (api.context_connect_fd)(context, fd.into_raw_fd(), null_mut(), 0);
    let stream = if core.is_null() {
        null_mut()
    } else {
        make_stream(api, core, name, data, events, hook)
    };
    if stream.is_null() {
        if !core.is_null() {
            (api.core_disconnect)(core);
        }
        (api.context_destroy)(context);
        (api.thread_loop_unlock)(thread_loop);
        (api.thread_loop_stop)(thread_loop);
        (api.thread_loop_destroy)(thread_loop);
        let step = if core.is_null() {
            "pw_context_connect_fd"
        } else {
            "pw_stream_connect"
        };
        return Err(fail(step));
    }
    (api.thread_loop_unlock)(thread_loop);
    Ok((thread_loop, context, core, stream))
}

/// The stream on `core`, connected to `node_id`; null on failure.
///
/// # Safety
/// As `create`, with the thread loop locked.
unsafe fn make_stream(
    api: &PipewireApi,
    core: *mut c_void,
    name: &CString,
    data: &mut CallbackData,
    events: &mut PwStreamEvents,
    hook: &mut SpaHook,
) -> *mut c_void {
    let keys = [
        "media.type",
        "Video",
        "media.category",
        "Capture",
        "media.role",
        "Screen",
        "node.dont-fallback",
        "true",
        "node.dont-move",
        "true",
        "node.dont-reconnect",
        "true",
    ]
    .map(|text| CString::new(text).unwrap_or_default());
    let (node_id, serial) = data.identity.target();
    let target = CString::new(serial.as_deref().unwrap_or("")).unwrap_or_default();
    let target_key = CString::new("target.object").unwrap_or_default();
    let properties = (api.properties_new)(
        keys[0].as_ptr(),
        keys[1].as_ptr(),
        keys[2].as_ptr(),
        keys[3].as_ptr(),
        keys[4].as_ptr(),
        keys[5].as_ptr(),
        keys[6].as_ptr(), keys[7].as_ptr(),
        keys[8].as_ptr(), keys[9].as_ptr(),
        keys[10].as_ptr(), keys[11].as_ptr(),
        if serial.is_some() { target_key.as_ptr() } else { null() },
        target.as_ptr(),
        null::<c_char>(),
    );
    if properties.is_null() {
        return null_mut();
    }
    let stream = (api.stream_new)(core, name.as_ptr(), properties);
    if stream.is_null() {
        return null_mut();
    }
    data.stream.store(stream, Ordering::Release);
    let data_pointer: *mut CallbackData = data;
    (api.stream_add_listener)(stream, hook, events, data_pointer.cast());
    // The pod lives in 8-byte words, as PipeWire reads it.
    let bytes = enum_format_pod();
    let mut words = vec![0u64; bytes.len().div_ceil(8)];
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), words.as_mut_ptr().cast::<u8>(), bytes.len());
    let mut params: [*const SpaPod; 1] = [words.as_ptr().cast()];
    let flags = PW_STREAM_FLAG_AUTOCONNECT | PW_STREAM_FLAG_MAP_BUFFERS
        | super::pipewire_abi::PW_STREAM_FLAG_INACTIVE | super::pipewire_abi::PW_STREAM_FLAG_DONT_RECONNECT;
    let result = (api.stream_connect)(
        stream,
        PW_DIRECTION_INPUT,
        node_id,
        flags,
        params.as_mut_ptr(),
        1,
    );
    if result < 0 {
        (api.stream_destroy)(stream);
        return null_mut();
    }
    stream
}

impl Drop for PwStream {
    fn drop(&mut self) {
        self._data.identity.invalidate();
        self._data.latest.detach();
        let api = &self.api;
        // SAFETY: the objects are ours and still alive; they go in the
        // reverse order of their creation, the loop stops after them.
        unsafe {
            (api.thread_loop_lock)(self.thread_loop);
            (api.stream_disconnect)(self.stream);
            (api.stream_destroy)(self.stream);
            (api.core_disconnect)(self.core);
            (api.context_destroy)(self.context);
            (api.thread_loop_unlock)(self.thread_loop);
            (api.thread_loop_stop)(self.thread_loop);
            (api.thread_loop_destroy)(self.thread_loop);
        }
    }
}
