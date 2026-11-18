//! The host's sound on Linux (plan 2026-11-17 P9 „Audio“) through the
//! simple API of PulseAudio (`libpulse-simple.so.0`, loaded at run time like
//! PipeWire in `pipewire_abi.rs`, so neither building nor starting needs
//! it). PipeWire answers the same API through `pipewire-pulse`, the default
//! sound server of current GNOME and KDE distributions, so one path covers
//! both servers.
//!
//! Capture: a record stream of `@DEFAULT_MONITOR@`, the monitor source of
//! the default sink (what the speakers play), 48 kHz stereo float, with a
//! fragment of one 5 ms frame, so each read returns one frame as soon as the
//! server has it. Playback: a playback stream on the default sink with a
//! target length of three frames (15 ms) and a start threshold of one;
//! `pa_simple_write` blocks while that is full, which paces the output.
//! The capture process of a root daemon runs as the followed session's
//! account with its variables (`XDG_RUNTIME_DIR` names its sound server).
use std::ffi::{c_char, c_int, c_void, CStr};
use std::io;
use std::process::Command;

use libloading::Library;

use crate::screen::core::audio_wire::{CHANNELS, FRAME_SAMPLES, SAMPLE_RATE};

pub const CAPTURE_AVAILABLE: bool = true;

/// `PA_SAMPLE_FLOAT32LE` of `pulse/sample.h`.
const SAMPLE_FLOAT32LE: c_int = 5;
/// `PA_STREAM_PLAYBACK` and `PA_STREAM_RECORD` of `pulse/def.h`.
const STREAM_PLAYBACK: c_int = 1;
const STREAM_RECORD: c_int = 2;
/// `(uint32_t) -1`: the server's default for a buffer field.
const DEFAULT: u32 = u32::MAX;
/// Bytes of one frame (240 samples, two channels, four bytes).
const FRAME_BYTES: u32 = (FRAME_SAMPLES * CHANNELS as usize * 4) as u32;

/// `pa_sample_spec`.
#[repr(C)]
struct SampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

/// `pa_buffer_attr`.
#[repr(C)]
struct BufferAttr {
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
    fragsize: u32,
}

type NewFn = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    c_int,
    *const c_char,
    *const c_char,
    *const SampleSpec,
    *const c_void,
    *const BufferAttr,
    *mut c_int,
) -> *mut c_void;
type FreeFn = unsafe extern "C" fn(*mut c_void);
type ReadFn = unsafe extern "C" fn(*mut c_void, *mut c_void, usize, *mut c_int) -> c_int;
type WriteFn = unsafe extern "C" fn(*mut c_void, *const c_void, usize, *mut c_int) -> c_int;

/// One stream of the simple API.
struct Simple {
    handle: *mut c_void,
    free: FreeFn,
    read: ReadFn,
    write: WriteFn,
    /// Keeps the functions above loaded; dropped last.
    _library: Library,
}

fn symbol<T: Copy>(library: &Library, name: &[u8]) -> io::Result<T> {
    // SAFETY: each caller names the C signature of the symbol as `T`.
    unsafe { library.get::<T>(name) }
        .map(|found| *found)
        .map_err(io::Error::other)
}

impl Simple {
    fn open(direction: c_int, device: Option<&CStr>, attr: &BufferAttr) -> io::Result<Self> {
        // SAFETY: loading the system library runs its initialisers, which
        // libpulse keeps free of side effects on this process.
        let library = unsafe { Library::new("libpulse-simple.so.0") }.map_err(|error| {
            io::Error::new(io::ErrorKind::NotFound, format!("libpulse-simple: {error}"))
        })?;
        let new: NewFn = symbol(&library, b"pa_simple_new\0")?;
        let free: FreeFn = symbol(&library, b"pa_simple_free\0")?;
        let read: ReadFn = symbol(&library, b"pa_simple_read\0")?;
        let write: WriteFn = symbol(&library, b"pa_simple_write\0")?;
        let spec = SampleSpec {
            format: SAMPLE_FLOAT32LE,
            rate: SAMPLE_RATE,
            // Two channels fit.
            channels: CHANNELS as u8,
        };
        let name = c"Smart Explorer";
        let stream = if direction == STREAM_RECORD {
            c"Bildschirm: Ton des Geräts"
        } else {
            c"Bildschirm: Ton der Gegenstelle"
        };
        let mut error: c_int = 0;
        // SAFETY: every pointer is valid for the call or null where the API
        // allows it (server, device, channel map).
        let handle = unsafe {
            new(
                std::ptr::null(),
                name.as_ptr(),
                direction,
                device.map_or(std::ptr::null(), CStr::as_ptr),
                stream.as_ptr(),
                &spec,
                std::ptr::null(),
                attr,
                &mut error,
            )
        };
        if handle.is_null() {
            return Err(io::Error::other(format!(
                "the sound server refused the stream (pulse error {error})"
            )));
        }
        Ok(Self {
            handle,
            free,
            read,
            write,
            _library: library,
        })
    }
}

impl Drop for Simple {
    fn drop(&mut self) {
        // SAFETY: the handle came from `pa_simple_new` and is freed once.
        unsafe { (self.free)(self.handle) };
    }
}

/// Capture of the default sink's monitor.
pub struct Capture {
    stream: Simple,
    frame: Vec<f32>,
}

impl Capture {
    pub fn open() -> io::Result<Self> {
        let attr = BufferAttr {
            maxlength: DEFAULT,
            tlength: DEFAULT,
            prebuf: DEFAULT,
            minreq: DEFAULT,
            fragsize: FRAME_BYTES,
        };
        let stream = Simple::open(STREAM_RECORD, Some(c"@DEFAULT_MONITOR@"), &attr)?;
        Ok(Self {
            stream,
            frame: vec![0.0; FRAME_SAMPLES * usize::from(CHANNELS)],
        })
    }

    pub fn format(&self) -> (u32, u16) {
        (SAMPLE_RATE, CHANNELS)
    }

    /// Appends one frame; blocks until the server has it.
    pub fn read(&mut self, out: &mut Vec<f32>) -> io::Result<()> {
        let mut error: c_int = 0;
        let bytes = std::mem::size_of_val(self.frame.as_slice());
        // SAFETY: `frame` has room for `bytes` bytes.
        let result = unsafe {
            (self.stream.read)(
                self.stream.handle,
                self.frame.as_mut_ptr().cast(),
                bytes,
                &mut error,
            )
        };
        if result < 0 {
            return Err(io::Error::other(format!(
                "sound capture failed (pulse error {error})"
            )));
        }
        out.extend_from_slice(&self.frame);
        Ok(())
    }
}

/// Playback on the default sink.
pub struct Output {
    stream: Simple,
}

impl Output {
    pub fn open() -> io::Result<Self> {
        let attr = BufferAttr {
            maxlength: DEFAULT,
            tlength: FRAME_BYTES * 3,
            prebuf: FRAME_BYTES,
            minreq: FRAME_BYTES,
            fragsize: DEFAULT,
        };
        Ok(Self {
            stream: Simple::open(STREAM_PLAYBACK, None, &attr)?,
        })
    }

    /// One frame at a time: `write` blocks while the stream is full.
    pub fn wait_space(&mut self) -> io::Result<usize> {
        Ok(FRAME_SAMPLES)
    }

    pub fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        let mut error: c_int = 0;
        // SAFETY: `samples` is readable for its byte length.
        let result = unsafe {
            (self.stream.write)(
                self.stream.handle,
                samples.as_ptr().cast(),
                std::mem::size_of_val(samples),
                &mut error,
            )
        };
        if result < 0 {
            return Err(io::Error::other(format!(
                "sound output failed (pulse error {error})"
            )));
        }
        Ok(())
    }
}

/// A root daemon starts the capture process in the followed session as
/// its account (like the screen host process, E23); a daemon of the
/// session's own account starts it unchanged.
pub fn prepare_helper(command: &mut Command) {
    use crate::screen::os::session_detect::active_session::spawn;
    if let Some(session) = spawn::for_worker() {
        spawn::apply(&session, command);
    }
}
