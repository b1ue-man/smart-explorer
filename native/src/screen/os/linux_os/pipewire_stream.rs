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
    PW_VERSION_STREAM_EVENTS, SPA_PARAM_FORMAT,
};
use super::spa_pod::{enum_format_pod, parse_video_format, VideoFormat};
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};

/// What `take` found.
pub(crate) enum Taken {
    Frame(BgraImage),
    Unchanged,
    /// The stream failed; the text names the cause.
    Failed(String),
}

#[derive(Default)]
struct LatestInner {
    seq: u64,
    taken: u64,
    image: Option<BgraImage>,
    failure: Option<String>,
}

/// The newest picture of the stream and its failure, shared between the
/// loop's thread and the capture.
#[derive(Default)]
pub(crate) struct LatestFrame {
    inner: Mutex<LatestInner>,
}

impl LatestFrame {
    fn lock(&self) -> MutexGuard<'_, LatestInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn publish(&self, image: BgraImage) {
        let mut inner = self.lock();
        inner.seq += 1;
        inner.image = Some(image);
    }

    pub(crate) fn fail(&self, reason: String) {
        self.lock().failure.get_or_insert(reason);
    }

    /// A picture newer than the last one taken; else `Unchanged`, but with
    /// `keyframe` the last picture again (a key frame always has one).
    pub(crate) fn take(&self, keyframe: bool) -> Taken {
        let mut inner = self.lock();
        if let Some(reason) = &inner.failure {
            return Taken::Failed(reason.clone());
        }
        let fresh = inner.seq != inner.taken;
        inner.taken = inner.seq;
        match &inner.image {
            Some(image) if fresh || keyframe => Taken::Frame(image.clone()),
            _ => Taken::Unchanged,
        }
    }
}

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
}

fn guarded(data: *mut c_void, work: impl FnOnce(&CallbackData)) {
    // SAFETY: `data` is the `CallbackData` box that `PwStream` keeps alive
    // until the stream is destroyed.
    let data = unsafe { &*(data as *const CallbackData) };
    let outcome = catch_unwind(AssertUnwindSafe(|| work(data)));
    if outcome.is_err() {
        data.latest.fail("pipewire_callback: panic".into());
    }
}

unsafe extern "C" fn on_state_changed(
    data: *mut c_void,
    _old: c_int,
    new: c_int,
    error: *const c_char,
) {
    guarded(data, |data| {
        if new == PW_STREAM_STATE_ERROR {
            let text = if error.is_null() {
                "unknown".to_string()
            } else {
                // SAFETY: PipeWire passes a NUL-terminated string.
                unsafe { CStr::from_ptr(error) }
                    .to_string_lossy()
                    .into_owned()
            };
            data.latest.fail(format!("pipewire_stream: {text}"));
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
            *lock_format(data) = None;
            return;
        }
        match negotiated {
            Some((format, size)) if size.is_valid_frame() => {
                *lock_format(data) = Some((format, size));
            }
            Some((_, size)) => data.latest.fail(format!(
                "pipewire_size: {}x{} is outside the 4096 x 2304 frame limit",
                size.width, size.height
            )),
            None => data.latest.fail("pipewire_format".into()),
        }
    });
}

fn lock_format(data: &CallbackData) -> MutexGuard<'_, Option<(VideoFormat, FrameSize)>> {
    data.format.lock().unwrap_or_else(PoisonError::into_inner)
}

unsafe extern "C" fn on_process(data: *mut c_void) {
    guarded(data, |data| {
        let stream = data.stream.load(Ordering::Acquire);
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
            if let Some(image) = copy_picture(data, last) {
                data.latest.publish(image);
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
    let (format, size) = (*lock_format(data))?;
    let spa = (*buffer).buffer;
    if spa.is_null() || (*spa).n_datas == 0 || (*spa).datas.is_null() {
        return None;
    }
    let block = &*(*spa).datas;
    if block.data.is_null() || block.chunk.is_null() {
        return None;
    }
    let chunk = &*block.chunk;
    if chunk.size == 0 {
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
        data.latest
            .fail("pipewire_buffer: offset beyond the buffer".into());
        return None;
    }
    let bytes = std::slice::from_raw_parts(block.data.cast::<u8>().add(offset), mapped - offset);
    match convert_rows(bytes, stride, format, size) {
        Ok(image) => Some(image),
        Err(error) => {
            data.latest.fail(error.to_string());
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
        node_id: u32,
        latest: Arc<LatestFrame>,
    ) -> io::Result<Self> {
        let name = CString::new("smart-explorer-screen").map_err(io::Error::other)?;
        let mut data = Box::new(CallbackData {
            api: api.clone(),
            latest,
            stream: AtomicPtr::new(null_mut()),
            format: Mutex::new(None),
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
            unsafe { create(&api, fd, node_id, &name, &mut data, &mut events, &mut hook) };
        let (thread_loop, context, core, stream) = created?;
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
    node_id: u32,
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
        make_stream(api, core, node_id, name, data, events, hook)
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
    node_id: u32,
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
    ]
    .map(|text| CString::new(text).unwrap_or_default());
    let properties = (api.properties_new)(
        keys[0].as_ptr(),
        keys[1].as_ptr(),
        keys[2].as_ptr(),
        keys[3].as_ptr(),
        keys[4].as_ptr(),
        keys[5].as_ptr(),
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
    let flags = PW_STREAM_FLAG_AUTOCONNECT | PW_STREAM_FLAG_MAP_BUFFERS;
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
