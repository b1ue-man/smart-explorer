//! The render streams of the host's sound on Windows (plan 2026-11-17 P9
//! „Audio“): the low-latency `IAudioClient3` stream of the viewer, the
//! silent `IAudioClient3` stream that keeps the host's engine at its
//! minimum period during a loopback capture, and the plain shared stream
//! both fall back to.
//!
//! `IAudioClient3::GetSharedModeEnginePeriod` names the periods the engine
//! takes for the mix format; `InitializeSharedAudioStream` opens a shared
//! stream at one of them, in the mix format and without the engine's
//! conversion (Microsoft, „Low Latency Audio“, WASAPI section). While such
//! a stream runs, every stream of the endpoint, the loopback capture
//! included, runs at its period (same article, FAQ).
use std::io;

use windows::Win32::Media::Audio::{
    IAudioClient, IAudioClient3, IAudioRenderClient, AUDCLNT_BUFFERFLAGS_SILENT,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX,
};
use windows::Win32::System::Com::CoTaskMemFree;

use super::{
    activate, describe, os_error, render_endpoint, stereo_float, Event, Samples, BUFFER_HNS,
};
use crate::screen::core::audio_convert::FromStereo48;
use crate::screen::core::audio_wire::{CHANNELS, FRAME_SAMPLES, SAMPLE_RATE};

/// Frames of `rate` as µs.
fn frames_us(frames: u32, rate: u32) -> u32 {
    let us = u64::from(frames) * 1_000_000 / u64::from(rate.max(1));
    u32::try_from(us).unwrap_or(u32::MAX)
}

/// The engine's mix format, freed with the struct.
struct MixFormat(*mut WAVEFORMATEX);

impl MixFormat {
    fn of(client: &IAudioClient3) -> io::Result<Self> {
        // SAFETY: the client is activated; the format is freed in `drop`.
        unsafe { client.GetMixFormat() }.map(Self).map_err(os_error)
    }
}

impl Drop for MixFormat {
    fn drop(&mut self) {
        // SAFETY: WASAPI allocated the format with CoTaskMemAlloc.
        unsafe { CoTaskMemFree(Some(self.0.cast_const().cast())) };
    }
}

/// A fresh `IAudioClient3` of the default render endpoint, its mix format
/// and the engine's smallest period for it, in frames.
fn low_client() -> io::Result<(IAudioClient3, MixFormat, u32)> {
    let client = activate::<IAudioClient3>()?;
    let mix = MixFormat::of(&client)?;
    let (mut default, mut fundamental, mut min, mut max) = (0u32, 0u32, 0u32, 0u32);
    // SAFETY: out-pointers to locals; the format lives in `mix`.
    unsafe {
        client.GetSharedModeEnginePeriod(mix.0, &mut default, &mut fundamental, &mut min, &mut max)
    }
    .map_err(os_error)?;
    if min == 0 {
        return Err(io::Error::other("the engine names no minimum period"));
    }
    Ok((client, mix, min))
}

/// Holds the render engine at its minimum period with a silent stream.
pub(super) struct PeriodKeeper {
    client: IAudioClient3,
    render: IAudioRenderClient,
    buffer: u32,
}

impl PeriodKeeper {
    pub(super) fn start() -> io::Result<Self> {
        let (client, mix, period) = low_client()?;
        // SAFETY: the format lives in `mix` across the calls; the buffer is
        // released with exactly the frames it was taken with.
        unsafe {
            client
                .InitializeSharedAudioStream(0, period, mix.0, None)
                .map_err(os_error)?;
            let buffer = client.GetBufferSize().map_err(os_error)?;
            let render = client
                .GetService::<IAudioRenderClient>()
                .map_err(os_error)?;
            let keeper = Self {
                client,
                render,
                buffer,
            };
            keeper.top_up();
            keeper.client.Start().map_err(os_error)?;
            Ok(keeper)
        }
    }

    /// Fills the stream with silence (the engine never starves it).
    pub(super) fn top_up(&self) {
        // SAFETY: the stream is initialised; the buffer is released with the
        // frames it was taken with, flagged silent (its bytes stay unread).
        unsafe {
            let Ok(queued) = self.client.GetCurrentPadding() else {
                return;
            };
            let free = self.buffer.saturating_sub(queued);
            if free > 0 && self.render.GetBuffer(free).is_ok() {
                let silent = AUDCLNT_BUFFERFLAGS_SILENT.0 as u32;
                let _ = self.render.ReleaseBuffer(free, silent);
            }
        }
    }
}

impl Drop for PeriodKeeper {
    fn drop(&mut self) {
        // SAFETY: stopping a started client; errors leave nothing to do.
        let _ = unsafe { self.client.Stop() };
    }
}

/// The viewer's low-latency stream: the mix format (32-bit float only) at
/// the minimum period, kept two periods ahead.
pub(super) struct LowRender {
    client: IAudioClient3,
    render: IAudioRenderClient,
    fill: u32,
    rate: u32,
    channels: u16,
    convert: FromStereo48,
    scratch: Vec<f32>,
}

impl LowRender {
    pub(super) fn open(event: &Event) -> io::Result<Self> {
        let (client, mix, period) = low_client()?;
        let (rate, channels, samples) = describe(mix.0)?;
        if samples != Samples::F32 {
            return Err(io::Error::other("the mix format is not 32-bit float"));
        }
        let convert = FromStereo48::new(rate, channels)?;
        // SAFETY: the format lives in `mix` across the call.
        let (render, buffer) = unsafe {
            client
                .InitializeSharedAudioStream(AUDCLNT_STREAMFLAGS_EVENTCALLBACK, period, mix.0, None)
                .map_err(os_error)?;
            client.SetEventHandle(event.handle()).map_err(os_error)?;
            let buffer = client.GetBufferSize().map_err(os_error)?;
            let render = client
                .GetService::<IAudioRenderClient>()
                .map_err(os_error)?;
            client.Start().map_err(os_error)?;
            (render, buffer)
        };
        Ok(Self {
            client,
            render,
            fill: period.saturating_mul(2).min(buffer),
            rate,
            channels,
            convert,
            scratch: Vec::new(),
        })
    }

    fn padding(&self) -> io::Result<u32> {
        // SAFETY: the client is started.
        unsafe { self.client.GetCurrentPadding() }.map_err(os_error)
    }

    /// 48 kHz stereo frames the device takes now.
    pub(super) fn free(&self) -> io::Result<usize> {
        let device = self.fill.saturating_sub(self.padding()?) as usize;
        Ok(self.convert.stereo_frames(device))
    }

    pub(super) fn queued_us(&self) -> u32 {
        frames_us(self.padding().unwrap_or(0), self.rate)
    }

    pub(super) fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        self.scratch.clear();
        self.convert.push(samples, &mut self.scratch);
        let channels = usize::from(self.channels);
        let frames = u32::try_from(self.scratch.len() / channels)
            .map_err(|_| io::Error::other("audio write too long"))?;
        if frames == 0 {
            return Ok(());
        }
        // SAFETY: `free` left room for at least `frames` frames of the mix
        // format (32-bit float, `channels` channels); exactly those are
        // written and released.
        unsafe {
            let buffer = self.render.GetBuffer(frames).map_err(os_error)?;
            let count = frames as usize * channels;
            std::ptr::copy_nonoverlapping(self.scratch.as_ptr(), buffer.cast::<f32>(), count);
            self.render.ReleaseBuffer(frames, 0).map_err(os_error)
        }
    }
}

impl Drop for LowRender {
    fn drop(&mut self) {
        // SAFETY: stopping a started client; errors leave nothing to do.
        let _ = unsafe { self.client.Stop() };
    }
}

/// The plain shared stream: 48 kHz stereo float converted by the engine,
/// kept one engine period and one 5 ms frame ahead.
pub(super) struct PlainRender {
    client: IAudioClient,
    render: IAudioRenderClient,
    fill: u32,
}

impl PlainRender {
    pub(super) fn open(event: &Event) -> io::Result<Self> {
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
            fill,
        })
    }

    fn padding(&self) -> io::Result<u32> {
        // SAFETY: the client is started.
        unsafe { self.client.GetCurrentPadding() }.map_err(os_error)
    }

    pub(super) fn free(&self) -> io::Result<usize> {
        Ok(self.fill.saturating_sub(self.padding()?) as usize)
    }

    pub(super) fn queued_us(&self) -> u32 {
        frames_us(self.padding().unwrap_or(0), SAMPLE_RATE)
    }

    pub(super) fn write(&mut self, samples: &[f32]) -> io::Result<()> {
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

impl Drop for PlainRender {
    fn drop(&mut self) {
        // SAFETY: stopping a started client; errors leave nothing to do.
        let _ = unsafe { self.client.Stop() };
    }
}
