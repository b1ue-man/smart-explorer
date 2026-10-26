//! The PipeWire video stream of a portal screen cast (SCR-M5, U19-5): joins
//! the portal's PipeWire remote with its file descriptor, connects an input
//! stream to the portal's node, negotiates raw video as shared memory
//! (`spa_pod::enum_format_pod`) and copies every picture into `LatestFrame`.
//! All calls on the stream and the core run between `lock` and `unlock` of a
//! `pw_thread_loop`; the callbacks run on the loop's thread. `capture` of
//! the source takes the picture from `LatestFrame` on another thread.
use super::pipewire_abi::{
    PipewireApi, PwBuffer, PwStreamEvents, SpaHook, SpaPod, PW_DIRECTION_INPUT,
    PW_STREAM_FLAG_AUTOCONNECT, PW_STREAM_STATE_ERROR, PW_VERSION_STREAM_EVENTS,
    SPA_CHUNK_FLAG_CORRUPTED, SPA_PARAM_FORMAT,
};
use super::spa_pod::{enum_format_pod, parse_video_format, VideoFormat};
use crate::screen::core::capture_admission::CaptureGrant;
use crate::screen::core::capture_image::{CaptureImageBuilder, CapturedImage};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::pixel_convert::FrameSize;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io;
use std::os::fd::{IntoRawFd, OwnedFd};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
#[path = "pipewire_capture_memory.rs"]
pub(crate) mod memory;
use memory::CaptureMemory;
#[path = "pipewire_buffers.rs"]
mod buffers;
use buffers::BufferOwners;
#[path = "pipewire_frame_copy.rs"]
mod frame_copy;
#[path = "pipewire_import.rs"]
mod imports;

#[path = "pipewire_trace.rs"]
mod trace;
use trace::Trace;
#[path = "pipewire_identity.rs"]
pub(super) mod identity;
use identity::StreamIdentity;

/// What `take` found.
pub(crate) enum Taken {
    Frame(CapturedImage),
    Unchanged,
    /// The stream failed; the text names the cause.
    Failed(String),
}

#[path = "pipewire_pause.rs"]
mod pause;
pub(crate) use pause::LatestFrame;

#[cfg(test)]
pub(crate) use memory::convert_rows;

/// What the callbacks need; boxed, so its address stays.
struct CallbackData {
    api: Arc<PipewireApi>,
    latest: Arc<LatestFrame>,
    stream: AtomicPtr<c_void>,
    format: Mutex<Option<(VideoFormat, FrameSize)>>,
    identity: Arc<StreamIdentity>,
    memory: Arc<CaptureMemory>,
    owners: Mutex<BufferOwners>,
    imports: Box<imports::ImportController>,
}

impl CallbackData {
    fn fail(&self, reason: String) {
        self.latest.fail(reason);
        self.identity.invalidate();
        self.memory.signal_stop();
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
        data.latest.on_state_changed(old, new);
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
            data.fail("portal_source_expired: choose the source again".into());
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
            let size = unsafe { (*param).size as usize };
            if size > 64 * 1024 {
                data.fail("pipewire format POD too large".into());
                return;
            }
            let bytes = unsafe { std::slice::from_raw_parts(param.cast::<u8>(), 8 + size) };
            parse_video_format(bytes)
        };
        if param.is_null() {
            data.latest.trace.format("none");
            data.latest.pending_format();
            *lock_format(data) = None;
            if let Err(error) = data.memory.format(None) {
                data.fail(error.to_string());
            }
            return;
        }
        match negotiated {
            Some((format, size)) if FrameLimits::RESOURCE.check(size).is_ok() => {
                data.latest
                    .trace
                    .format(&format!("{format:?} {}x{}", size.width, size.height));
                data.latest.pending_format();
                *lock_format(data) = Some((format, size));
                if let Err(error) = data.memory.format(Some((format, size))) {
                    data.fail(error.to_string());
                    return;
                }
                // Functional size request only: native imports are separately
                // admitted at Core.add_mem before their first local mapping.
                let bytes = match super::spa_pod::consumer_buffers_pod(size) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        data.fail(error.to_string());
                        return;
                    }
                };
                let mut words = vec![0u64; bytes.len().div_ceil(8)];
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        bytes.as_ptr(),
                        words.as_mut_ptr().cast::<u8>(),
                        bytes.len(),
                    );
                }
                let mut params = [words.as_ptr().cast::<SpaPod>()];
                let result = unsafe {
                    (data.api.stream_update_params)(
                        data.stream.load(Ordering::Acquire),
                        params.as_mut_ptr(),
                        1,
                    )
                };
                if result < 0 {
                    data.fail(format!(
                        "pipewire functional buffer request failed: {result}"
                    ));
                }
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
        if stream.is_null() {
            return;
        }
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
                if let Some(image) = copy_picture(data, last) {
                    data.latest.publish(image);
                }
            }
            (data.api.stream_queue_buffer)(stream, last);
        }
    });
}

use buffers::{on_add_buffer, on_remove_buffer};
use frame_copy::copy_picture;
/// A running stream; dropping it stops and destroys everything.
pub(crate) struct PwStream {
    api: Arc<PipewireApi>,
    thread_loop: *mut c_void,
    context: *mut c_void,
    // Kept for the callbacks until the stream is destroyed.
    _hook: Box<SpaHook>,
    _events: Box<PwStreamEvents>,
    _data: Box<CallbackData>,
    control: Option<std::thread::JoinHandle<()>>,
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
        memory: Arc<CaptureMemory>,
    ) -> io::Result<Self> {
        // stream.c permits PIPEWIRE_NODE to overwrite target.object. A
        // portal capture must never honor that process-wide source override.
        buffers::check_target_override(std::env::var_os("PIPEWIRE_NODE").as_deref())?;
        let name = CString::new("smart-explorer-screen").map_err(io::Error::other)?;
        identity.attach(Arc::downgrade(&latest));
        if identity.expired() {
            return Err(io::Error::other("portal_source_expired"));
        }
        latest.suspend_before_connect();
        let imports = imports::ImportController::new(api.memory, latest.clone(), memory.clone())?;
        let mut data = Box::new(CallbackData {
            api: api.clone(),
            latest,
            stream: AtomicPtr::new(null_mut()),
            format: Mutex::new(None),
            identity,
            memory,
            owners: Mutex::new(BufferOwners::default()),
            imports,
        });
        let mut events = Box::new(PwStreamEvents {
            version: PW_VERSION_STREAM_EVENTS,
            destroy: Some(on_destroy),
            state_changed: Some(on_state_changed),
            control_info: None,
            io_changed: None,
            param_changed: Some(on_param_changed),
            add_buffer: Some(on_add_buffer),
            remove_buffer: Some(on_remove_buffer),
            process: Some(on_process),
            drained: None,
            command: None,
            trigger_done: None,
        });
        let mut hook = Box::new(SpaHook::zeroed());
        // SAFETY: the calls follow the documented PipeWire order; every
        // object is checked for null and given back on failure.
        let created =
            unsafe { imports::control::create(&api, fd, &name, &mut data, &mut events, &mut hook) };
        let (thread_loop, context, _core, _stream) = created?;
        let attached = unsafe {
            (api.thread_loop_lock)(thread_loop);
            let current = data.stream.load(Ordering::Acquire);
            let valid = !current.is_null() && !data.identity.expired();
            if valid {
                data.latest.attach(api.clone(), thread_loop, current);
            }
            (api.thread_loop_unlock)(thread_loop);
            valid
        };
        if !attached {
            let temporary = Self {
                api,
                thread_loop,
                context,
                _hook: hook,
                _events: events,
                _data: data,
                control: None,
            };
            drop(temporary);
            return Err(io::Error::other("pipewire source closed during connection"));
        }
        let pointer = (&*data as *const CallbackData) as usize;
        let loop_address = thread_loop as usize;
        let control = imports::control::start(pointer, loop_address);
        let control = match control {
            Ok(control) => control,
            Err(error) => {
                let temporary = Self {
                    api,
                    thread_loop,
                    context,
                    _hook: hook,
                    _events: events,
                    _data: data,
                    control: None,
                };
                drop(temporary);
                return Err(error);
            }
        };
        Ok(Self {
            api,
            thread_loop,
            context,
            _hook: hook,
            _events: events,
            _data: data,
            control: Some(control),
        })
    }
}

unsafe extern "C" fn on_destroy(pointer: *mut c_void) {
    guarded(pointer, |data| {
        data.stream.store(null_mut(), Ordering::Release);
        data.latest.stream_destroyed();
        data.fail("pipewire_stream_destroyed".into());
    });
}
impl Drop for PwStream {
    fn drop(&mut self) {
        self._data.identity.invalidate();
        self._data.latest.close_copy();
        self._data.imports.close();
        self._data.memory.stop();
        // Both Parent frame and import waits are cancelled before join. Never
        // hold loop/control locks while joining the authority thread.
        if let Some(control) = self.control.take() {
            let _ = control.join();
        }
        self._data.latest.detach();
        let api = &self.api;
        unsafe {
            (api.thread_loop_lock)(self.thread_loop);
            let stream = self._data.stream.load(Ordering::Acquire);
            if !stream.is_null() {
                (api.stream_set_active)(stream, false);
                (api.stream_disconnect)(stream);
                (api.stream_destroy)(stream);
            }
            self._data
                .owners
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
            // Release our whole-map handles after every stream access ended.
            self._data.imports.reap_release_handles();
            let core = self._data.imports.core.load(Ordering::Acquire);
            if !core.is_null() {
                (api.core_disconnect)(core);
            }
            self._data.imports.finish_core();
            (api.context_destroy)(self.context);
            (api.thread_loop_unlock)(self.thread_loop);
            (api.thread_loop_stop)(self.thread_loop);
            (api.thread_loop_destroy)(self.thread_loop);
        }
    }
}
