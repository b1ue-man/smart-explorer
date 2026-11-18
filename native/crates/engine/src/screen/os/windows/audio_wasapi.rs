//! The host's sound on Windows (plan 2026-11-17 P9 „Audio“) through WASAPI
//! in shared mode, as Sunshine captures and Moonlight plays on Windows.
//!
//! Capture: loopback of the default render endpoint (`eRender`,
//! `eConsole`). It asks for 48 kHz stereo float with
//! `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM` first (the audio engine converts);
//! where that is refused it takes the mix format and the caller converts
//! (`core/audio_convert.rs`). The capture waits on its event
//! (`AUDCLNT_STREAMFLAGS_EVENTCALLBACK`, loopback since Windows 10) and,
//! should the event never come, polls every `WAIT_MS`.
//!
//! Playback: the default render endpoint, 48 kHz stereo float with
//! `AUTOCONVERTPCM`, event driven; the device is kept filled one engine
//! period plus one 5 ms frame ahead, the least that survives one engine
//! period between two refills.
use std::io;
use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::core::GUID;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IAudioRenderClient, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX,
    WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED,
};

use crate::screen::core::audio_wire::{CHANNELS, FRAME_SAMPLES, SAMPLE_RATE};

pub const CAPTURE_AVAILABLE: bool = true;

/// `mmreg.h` format tags.
const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
/// `KSDATAFORMAT_SUBTYPE_PCM` and `KSDATAFORMAT_SUBTYPE_IEEE_FLOAT`.
const SUBTYPE_PCM: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
const SUBTYPE_IEEE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
/// `SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT`.
const STEREO_MASK: u32 = 0x3;
/// Size of the extension after `WAVEFORMATEX` in `WAVEFORMATEXTENSIBLE`.
const EXTENSIBLE_EXTRA: u16 = 22;
/// Buffer of the streams in 100-ns units (20 ms): the engine's period is
/// the delay, this is only the room.
const BUFFER_HNS: i64 = 200_000;
/// Longest wait for an event before the buffer is looked at anyway.
const WAIT_MS: u32 = 20;

fn os_error(error: windows::core::Error) -> io::Error {
    io::Error::other(error)
}

/// An auto-reset event of the stream.
struct Event(windows_sys::Win32::Foundation::HANDLE);

impl Event {
    fn new() -> io::Result<Self> {
        // SAFETY: default security, auto reset, not signalled, no name.
        let handle = unsafe {
            windows_sys::Win32::System::Threading::CreateEventW(
                std::ptr::null(),
                0,
                0,
                std::ptr::null(),
            )
        };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(handle))
    }

    fn handle(&self) -> HANDLE {
        HANDLE(self.0)
    }

    /// Waits for the event or `WAIT_MS`.
    fn wait(&self) {
        // SAFETY: the handle stays open for the life of `self`.
        unsafe { windows_sys::Win32::System::Threading::WaitForSingleObject(self.0, WAIT_MS) };
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: the handle was created by `Event::new` and is closed once.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

/// COM on this thread (multithreaded apartment).
fn com() -> io::Result<()> {
    // SAFETY: no reserved pointer; S_FALSE (already initialised) is fine.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(os_error)
}

/// A fresh client of the default render endpoint.
fn render_endpoint() -> io::Result<IAudioClient> {
    // SAFETY: COM is initialised on this thread (`com`); the results are
    // owned interfaces.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(os_error)?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(os_error)?;
        device
            .Activate::<IAudioClient>(CLSCTX_ALL, None)
            .map_err(os_error)
    }
}

/// 48 kHz stereo float.
fn stereo_float() -> WAVEFORMATEXTENSIBLE {
    let block = CHANNELS * 4;
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE,
            nChannels: CHANNELS,
            nSamplesPerSec: SAMPLE_RATE,
            nAvgBytesPerSec: SAMPLE_RATE * u32::from(block),
            nBlockAlign: block,
            wBitsPerSample: 32,
            cbSize: EXTENSIBLE_EXTRA,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: 32,
        },
        dwChannelMask: STEREO_MASK,
        SubFormat: SUBTYPE_IEEE_FLOAT,
    }
}

/// How the samples of a format are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Samples {
    F32,
    I16,
    I24,
    I32,
}

/// Rate, channels and sample kind of a format WASAPI gave.
fn describe(format: *const WAVEFORMATEX) -> io::Result<(u32, u16, Samples)> {
    // SAFETY: WASAPI returned a valid format; the structs are packed, so
    // they are read unaligned and their fields copied out.
    let base = unsafe { std::ptr::read_unaligned(format) };
    let (tag, channels, rate, bits, extra) = (
        base.wFormatTag,
        base.nChannels,
        base.nSamplesPerSec,
        base.wBitsPerSample,
        base.cbSize,
    );
    let float = if tag == WAVE_FORMAT_EXTENSIBLE && extra >= EXTENSIBLE_EXTRA {
        // SAFETY: `cbSize` says the extension follows.
        let extended = unsafe { std::ptr::read_unaligned(format.cast::<WAVEFORMATEXTENSIBLE>()) };
        let sub = extended.SubFormat;
        if sub != SUBTYPE_IEEE_FLOAT && sub != SUBTYPE_PCM {
            return Err(io::Error::other("audio mix format has an unknown subtype"));
        }
        sub == SUBTYPE_IEEE_FLOAT
    } else if tag == WAVE_FORMAT_IEEE_FLOAT || tag == WAVE_FORMAT_PCM {
        tag == WAVE_FORMAT_IEEE_FLOAT
    } else {
        return Err(io::Error::other("audio mix format has an unknown tag"));
    };
    let samples = match (float, bits) {
        (true, 32) => Samples::F32,
        (false, 16) => Samples::I16,
        (false, 24) => Samples::I24,
        (false, 32) => Samples::I32,
        _ => {
            return Err(io::Error::other(
                "audio mix format has an unsupported sample size",
            ))
        }
    };
    Ok((rate, channels, samples))
}

impl Samples {
    fn bytes(self) -> usize {
        match self {
            Self::F32 | Self::I32 => 4,
            Self::I16 => 2,
            Self::I24 => 3,
        }
    }
}

/// Appends the samples of `kind` in `bytes` as float.
fn append(bytes: &[u8], kind: Samples, out: &mut Vec<f32>) {
    out.extend(bytes.chunks_exact(kind.bytes()).map(|sample| match kind {
        Samples::F32 => f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]),
        Samples::I16 => f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32_768.0,
        Samples::I24 => {
            let value = i32::from_le_bytes([0, sample[0], sample[1], sample[2]]) >> 8;
            value as f32 / 8_388_608.0
        }
        Samples::I32 => {
            i32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]) as f32
                / 2_147_483_648.0
        }
    }));
}

/// Loopback capture of the default render endpoint.
pub struct Capture {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    event: Event,
    rate: u32,
    channels: u16,
    samples: Samples,
}

impl Capture {
    pub fn open() -> io::Result<Self> {
        com()?;
        let event = Event::new()?;
        let loopback = AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
        // 48 kHz stereo float converted by the engine first.
        let wanted = stereo_float();
        let client = render_endpoint()?;
        let converted =
            loopback | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        // SAFETY: `wanted` lives across the call; WASAPI copies the format.
        let direct = unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                converted,
                BUFFER_HNS,
                0,
                std::ptr::from_ref(&wanted).cast::<WAVEFORMATEX>(),
                None,
            )
        };
        let (client, (rate, channels, samples)) = match direct {
            Ok(()) => (client, (SAMPLE_RATE, CHANNELS, Samples::F32)),
            Err(_) => {
                // The mix format, converted here.
                let client = render_endpoint()?;
                // SAFETY: the mix format is freed below with CoTaskMemFree.
                let mix = unsafe { client.GetMixFormat() }.map_err(os_error)?;
                let described = describe(mix);
                // SAFETY: `mix` lives until it is freed after the call.
                let initialized = unsafe {
                    client.Initialize(AUDCLNT_SHAREMODE_SHARED, loopback, BUFFER_HNS, 0, mix, None)
                };
                // SAFETY: WASAPI allocated `mix` with CoTaskMemAlloc.
                unsafe { CoTaskMemFree(Some(mix.cast_const().cast())) };
                initialized.map_err(os_error)?;
                (client, described?)
            }
        };
        // SAFETY: the client is initialised with the event flag.
        let capture = unsafe {
            client.SetEventHandle(event.handle()).map_err(os_error)?;
            let capture = client
                .GetService::<IAudioCaptureClient>()
                .map_err(os_error)?;
            client.Start().map_err(os_error)?;
            capture
        };
        Ok(Self {
            client,
            capture,
            event,
            rate,
            channels,
            samples,
        })
    }

    /// Rate and channels of what `read` delivers.
    pub fn format(&self) -> (u32, u16) {
        (self.rate, self.channels)
    }

    /// Appends the next captured samples (interleaved, device format as
    /// float); waits while nothing plays.
    pub fn read(&mut self, out: &mut Vec<f32>) -> io::Result<()> {
        let channels = usize::from(self.channels);
        loop {
            // SAFETY: the capture client belongs to the started stream.
            let next = unsafe { self.capture.GetNextPacketSize() }.map_err(os_error)?;
            if next == 0 {
                if !out.is_empty() {
                    return Ok(());
                }
                self.event.wait();
                continue;
            }
            let mut data = std::ptr::null_mut();
            let mut frames = 0u32;
            let mut flags = 0u32;
            // SAFETY: out-pointers to locals; the buffer is released below.
            unsafe {
                self.capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                    .map_err(os_error)?;
            }
            let count = frames as usize * channels;
            if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null() {
                out.resize(out.len() + count, 0.0);
            } else {
                // SAFETY: WASAPI hands `frames` frames of the stream format,
                // readable until `ReleaseBuffer`.
                let bytes = unsafe {
                    std::slice::from_raw_parts(data.cast_const(), count * self.samples.bytes())
                };
                append(bytes, self.samples, out);
            }
            // SAFETY: releases exactly the frames `GetBuffer` gave.
            unsafe { self.capture.ReleaseBuffer(frames) }.map_err(os_error)?;
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        // SAFETY: stopping a started client; errors leave nothing to do.
        let _ = unsafe { self.client.Stop() };
    }
}

/// Playback on the default render endpoint.
pub struct Output {
    client: IAudioClient,
    render: IAudioRenderClient,
    event: Event,
    /// Frames kept in the device: one engine period and one 5 ms frame.
    fill: u32,
}

impl Output {
    pub fn open() -> io::Result<Self> {
        com()?;
        let event = Event::new()?;
        let client = render_endpoint()?;
        let format = stereo_float();
        let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
            | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        // SAFETY: `format` lives across the calls; out-pointers to locals.
        let (render, fill) = unsafe {
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    flags,
                    BUFFER_HNS,
                    0,
                    std::ptr::from_ref(&format).cast::<WAVEFORMATEX>(),
                    None,
                )
                .map_err(os_error)?;
            client.SetEventHandle(event.handle()).map_err(os_error)?;
            let buffer = client.GetBufferSize().map_err(os_error)?;
            let mut period_hns = 0i64;
            client
                .GetDevicePeriod(Some(std::ptr::from_mut(&mut period_hns)), None)
                .map_err(os_error)?;
            let period = u32::try_from(period_hns.max(0) * i64::from(SAMPLE_RATE) / 10_000_000)
                .unwrap_or(u32::MAX);
            let render = client
                .GetService::<IAudioRenderClient>()
                .map_err(os_error)?;
            let frame = u32::try_from(FRAME_SAMPLES).unwrap_or(u32::MAX);
            (render, period.saturating_add(frame).min(buffer))
        };
        // SAFETY: the client is initialised.
        unsafe { client.Start() }.map_err(os_error)?;
        Ok(Self {
            client,
            render,
            event,
            fill,
        })
    }

    /// Waits until the device takes frames; how many (per channel).
    pub fn wait_space(&mut self) -> io::Result<usize> {
        loop {
            // SAFETY: the client is started.
            let queued = unsafe { self.client.GetCurrentPadding() }.map_err(os_error)?;
            if queued < self.fill {
                return Ok((self.fill - queued) as usize);
            }
            self.event.wait();
        }
    }

    /// Writes interleaved 48 kHz stereo; at most what `wait_space` gave.
    pub fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        let frames = u32::try_from(samples.len() / usize::from(CHANNELS))
            .map_err(|_| io::Error::other("audio write too long"))?;
        if frames == 0 {
            return Ok(());
        }
        // SAFETY: the render buffer has room for `frames` frames of the
        // stream format (32-bit float stereo); exactly those are written
        // and released.
        unsafe {
            let buffer = self.render.GetBuffer(frames).map_err(os_error)?;
            let count = frames as usize * usize::from(CHANNELS);
            std::ptr::copy_nonoverlapping(samples.as_ptr(), buffer.cast::<f32>(), count);
            self.render.ReleaseBuffer(frames, 0).map_err(os_error)
        }
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        // SAFETY: stopping a started client; errors leave nothing to do.
        let _ = unsafe { self.client.Stop() };
    }
}

/// The capture process starts without a console window.
pub fn prepare_helper(command: &mut Command) {
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
}
