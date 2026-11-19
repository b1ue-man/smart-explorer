//! Hardware H.264 encoders of Media Foundation (asynchronous MFTs; SCR-M3,
//! decision C2): unlocked with `MF_TRANSFORM_ASYNC_UNLOCK` and driven by
//! their events (`METransformNeedInput`, `METransformHaveOutput`), which are
//! polled against a deadline, so a stuck driver ends in an error instead of
//! a hang. In low-latency mode each input gives its output before the next
//! input; an output that arrives without an input of this frame is an
//! encoder failure (a lost reference would corrupt the stream).
//!
//! Plan 2026-11-14 bildkette (package BK2a): with an admitted capture the
//! MFT gets the D3D11 device manager of the capture (B-20) and textures as
//! input (`mf_d3d_input.rs`; conversion and scaling on the GPU in
//! `d3d_video_convert.rs`, B-17), the settings of B-21 and a bit rate bounded
//! by what the encoder and its level report (`mf_media.rs`). A GPU failure
//! in operation switches this encoder to NV12 in system memory (manager
//! taken back, streaming restarted, key frame) and the capture of the
//! session to the CPU path; the frame in between is skipped and the next one
//! asked for (`needs_refresh`).
//!
//! Plan 2026-11-17 „Windows-Bild“ P3: a D3D11-aware MFT whose capture runs
//! on no or another adapter gets a device of its own on its adapter with
//! the CPU upload (`d3d_input::attach_own`, (b)); its failure does not
//! switch the capture's route. Key frames follow rule D8
//! (`mf_capture_output.rs`).
use std::io;
use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Win32::Media::MediaFoundation::{
    ICodecAPI, IMFMediaEventGenerator, IMFMediaType, IMFTransform, MEError, METransformHaveOutput,
    METransformNeedInput, MFT_MESSAGE_COMMAND_FLUSH, MF_EVENT_FLAG_NO_WAIT,
    MF_E_NO_EVENTS_AVAILABLE, MF_TRANSFORM_ASYNC_UNLOCK,
};

use super::mf_common::{ensure_com, EncoderSetup};
use super::mf_encoder_sync::{force_keyframe, start, stop, UNITS_PER_SECOND};
use super::mf_media::{
    bounded_bitrate, configure_hardware, is_keyframe, mark_ltr, opened_limit, refused_lowering,
    retune_bounded, sharp_mode, Output,
};
use super::{mf_capture_media, mf_capture_output};
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner};
use crate::screen::core::capture_image::{AdmittedBytes, CapturedImage};
use crate::screen::core::capture_picture::Picture;
use crate::screen::core::host_backend::{CapturedEncodedFrame, EncodedFrame, ScreenEncoder};
use crate::screen::core::host_stage::StageRequest;
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::rate_control::honour::EncoderRateState;
use crate::screen::core::rate_control::START_DIRECT_BPS;
use d3d_input::InputRoute;

#[cfg(test)]
#[path = "bildkette_gpu_task_tests.rs"]
mod bildkette_gpu_task_tests;
#[path = "mf_d3d_input.rs"]
pub(super) mod d3d_input;
#[path = "mf_encoder_plain.rs"]
mod plain;
#[path = "d3d_video_convert.rs"]
pub(super) mod video_convert;

/// Longest wait for one event of the transform.
const EVENT_DEADLINE: Duration = Duration::from_secs(2);
/// Pause between two polls of the event queue.
const EVENT_POLL: Duration = Duration::from_micros(250);

enum Event {
    NeedInput,
    HaveOutput,
    Other,
}

pub(super) struct AsyncEncoder {
    transform: IMFTransform,
    events: IMFMediaEventGenerator,
    api: Option<ICodecAPI>,
    setup: EncoderSetup,
    frames: i64,
    bitrate: u32,
    /// `METransformNeedInput` events not answered with an input yet.
    credits: u32,
    grant: Option<CaptureGrant>,
    /// Textures as input (B-20); `None`: NV12 in system memory.
    gpu: Option<d3d_input::GpuInput>,
    /// Bit rate ceiling of the opened encoder and its level.
    limit: u32,
    /// The encoder controls long-term references (key frames are LTR 0).
    ltr: bool,
    /// A frame was skipped: the next request repeats the newest picture.
    refresh: bool,
    /// Streaming restarted: the next frame must be a key frame.
    restarted: bool,
    /// The MFT refused its mean bit rate or reads back another one (plan
    /// 2026-11-17, `ScreenEncoder::rate_state`).
    rate_refused: bool,
    /// Sharp mode as peak-constrained VBR (`mf_rate::peak_mode`, B2).
    peak: bool,
    /// D8: the last parameter sets and those of the output type.
    parameter_sets: Option<Vec<u8>>,
    sequence_header: Option<Vec<u8>>,
}

// SAFETY: as `SyncEncoder`: one thread at a time, free-threaded objects of
// the multithreaded apartment; the D3D11 objects of `gpu` are used under
// the device's multithread protection.
unsafe impl Send for AsyncEncoder {}

impl AsyncEncoder {
    pub(super) fn new(transform: IMFTransform, setup: EncoderSetup) -> io::Result<Self> {
        Self::create(transform, setup, None, START_DIRECT_BPS)
    }

    /// Opens at `bps`, the rate the session gives it (plan 2026-11-17).
    pub(super) fn new_admitted(
        transform: IMFTransform,
        setup: EncoderSetup,
        grant: CaptureGrant,
        bps: u64,
    ) -> io::Result<Self> {
        Self::create(transform, setup, Some(grant), bps)
    }

    fn create(
        transform: IMFTransform,
        setup: EncoderSetup,
        grant: Option<CaptureGrant>,
        bps: u64,
    ) -> io::Result<Self> {
        // SAFETY: attribute write on the activated transform; without it an
        // asynchronous MFT refuses every call.
        unsafe {
            transform
                .GetAttributes()?
                .SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?
        };
        let events: IMFMediaEventGenerator = transform.cast()?;
        // The manager goes to the MFT before its media types (B-20); only an
        // admitted capture has a device to share. Without one on its adapter
        // (none yet, another adapter, no admission) a device of its own (P3).
        let attached = match grant {
            Some(_) => d3d_input::attach(&transform, setup.size, setup.adapter),
            None => Err(d3d_input::SystemReason::NoCapture),
        };
        let mut gpu = attached
            .or_else(|reason| match d3d_input::input_route(Err(reason), true) {
                InputRoute::OwnDevice => {
                    d3d_input::attach_own(&transform, setup.size, setup.adapter)
                }
                _ => Err(reason),
            })
            .map_err(|reason| {
                eprintln!(
                    "screen encoder {}: system memory input ({reason:?})",
                    setup.name
                );
            })
            .ok();
        let bitrate = bounded_bitrate(bps, u32::MAX);
        let (api, settings) = configure_hardware(&transform, bitrate, setup.fps);
        // P9: sharp mode of an admitted session, where the MFT takes it, as
        // peak-constrained VBR under the session rate (review 2026-11-19 B2:
        // the quality mode kept no rate, the user's ceiling included).
        let quality = grant
            .as_ref()
            .and_then(|_| crate::screen::core::rate_control::model::session_quality());
        let peak = sharp_mode(api.as_ref(), quality.is_some(), bitrate, &setup.name)?;
        if let Err(error) = start(&transform, &setup, bitrate) {
            if gpu.is_none() {
                return Err(error);
            }
            // The types were refused with the manager: system memory.
            eprintln!(
                "screen encoder {}: texture input refused: {error}",
                setup.name
            );
            gpu = None;
            d3d_input::detach(&transform);
            start(&transform, &setup, bitrate)?;
        }
        let limit = opened_limit(&transform, api.as_ref(), &setup);
        let sequence_header = mf_capture_output::sequence_header(&transform);
        let refused = settings.refused();
        let rate_refused = refused.contains(&super::mf_media::SETTINGS[3]);
        if !refused.is_empty() {
            eprintln!(
                "screen encoder {}: settings at the encoder's default: {}",
                setup.name,
                refused.join(", ")
            );
        }
        Ok(Self {
            transform,
            events,
            api,
            setup,
            frames: 0,
            bitrate,
            credits: 0,
            grant,
            gpu,
            limit,
            ltr: settings.ltr_frames > 0,
            refresh: false,
            restarted: false,
            rate_refused,
            peak,
            parameter_sets: None,
            sequence_header,
        })
    }

    /// The next event of the transform, at the latest at `deadline`.
    fn next_event(&self, deadline: Instant) -> io::Result<Event> {
        loop {
            // SAFETY: a non-blocking poll of the transform's event queue.
            match unsafe { self.events.GetEvent(MF_EVENT_FLAG_NO_WAIT) } {
                Ok(event) => {
                    // SAFETY: reads of an owned event.
                    let (kind, status) = unsafe { (event.GetType()?, event.GetStatus()?) };
                    status.ok().map_err(io::Error::other)?;
                    return Ok(if kind == METransformNeedInput.0 as u32 {
                        Event::NeedInput
                    } else if kind == METransformHaveOutput.0 as u32 {
                        Event::HaveOutput
                    } else if kind == MEError.0 as u32 {
                        return Err(io::Error::other("the hardware encoder reported an error"));
                    } else {
                        Event::Other
                    });
                }
                Err(error) if error.code() == MF_E_NO_EVENTS_AVAILABLE => {
                    if Instant::now() >= deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "the hardware encoder did not answer in time",
                        ));
                    }
                    std::thread::sleep(EVENT_POLL);
                }
                Err(error) => return Err(io::Error::other(error)),
            }
        }
    }

    /// Waits for `METransformNeedInput` unless a credit is left.
    fn await_input(&mut self, deadline: Instant) -> io::Result<()> {
        while self.credits == 0 {
            match self.next_event(deadline)? {
                Event::NeedInput => {
                    self.credits = self
                        .credits
                        .checked_add(1)
                        .ok_or_else(|| io::Error::other("MF input credit overflow"))?
                }
                Event::HaveOutput => {
                    return Err(io::Error::other(
                        "the hardware encoder gave a picture without its frame",
                    ))
                }
                Event::Other => (),
            }
        }
        Ok(())
    }

    fn encode_once(
        &mut self,
        grant: &CaptureGrant,
        image: &Picture,
        request: &StageRequest,
        maximum: usize,
    ) -> io::Result<CapturedEncodedFrame> {
        let deadline = Instant::now() + EVENT_DEADLINE;
        self.await_input(deadline)?;
        let duration = UNITS_PER_SECOND / i64::from(self.setup.fps.max(1));
        let time = self
            .frames
            .checked_mul(duration)
            .ok_or_else(|| io::Error::other("MF frame timestamp overflow"))?;
        let sample = match self.gpu.as_mut() {
            Some(gpu) => gpu.sample(grant, image, time, duration)?,
            None => mf_capture_media::input_sample(grant, image, time, duration)?,
        };
        self.frames = self
            .frames
            .checked_add(1)
            .ok_or_else(|| io::Error::other("MF frame count overflow"))?;
        // SAFETY: one owned input sample for stream 0 after `NeedInput`.
        unsafe { self.transform.ProcessInput(0, &sample, 0)? };
        self.credits -= 1;
        drop(sample);
        let mut bytes = AdmittedBytes::buffer(grant, CaptureOwner::CodecOutput, maximum as u64)?;
        loop {
            match self.next_event(deadline)? {
                Event::NeedInput => {
                    self.credits = self
                        .credits
                        .checked_add(1)
                        .ok_or_else(|| io::Error::other("MF input credit overflow"))?
                }
                Event::HaveOutput => match mf_capture_output::process_output(
                    &self.transform,
                    grant,
                    self.setup.size.bgra_len(),
                )? {
                    Output::Sample(sample) => {
                        // The input texture is consumed with its output.
                        if let Some(gpu) = self.gpu.as_mut() {
                            gpu.finish();
                        }
                        let keyframe = is_keyframe(&sample);
                        mf_capture_output::append_unit_sample(
                            &mut bytes,
                            &sample,
                            self.setup.codec,
                            &mut self.parameter_sets,
                            self.sequence_header.as_deref(),
                        )?;
                        let wanted = request.keyframe || self.restarted;
                        let (codec, size) = (self.setup.codec, self.setup.size);
                        return mf_capture_output::finish(bytes, codec, size, keyframe, wanted);
                    }
                    Output::NeedInput => (),
                    Output::StreamChange => {
                        return Err(io::Error::other("the encoder changed its output type"))
                    }
                },
                Event::Other => (),
            }
        }
    }

    /// The GPU path failed: CPU path for the session, this encoder on NV12
    /// in system memory; else the original error (the next encoder follows).
    fn gpu_failed(&mut self, error: io::Error) -> io::Result<CapturedEncodedFrame> {
        // A device of its own is not the capture's (P3).
        if self.gpu.as_ref().is_some_and(|gpu| !gpu.is_own()) {
            d3d_input::disable(&format!("{}: {error}", self.setup.name));
        }
        self.gpu = None;
        if self.restart_system().is_err() {
            return Err(error);
        }
        self.refresh = true;
        self.restarted = true;
        Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "the hardware encoder takes system memory from the next picture",
        ))
    }

    /// Ends the stream, takes the device manager back and starts anew.
    fn restart_system(&mut self) -> io::Result<()> {
        stop(&self.transform);
        // SAFETY: messages and type resets on the activated transform; the
        // types are set again by `start`.
        unsafe {
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
            let _ = self.transform.SetInputType(0, None::<&IMFMediaType>, 0);
            let _ = self.transform.SetOutputType(0, None::<&IMFMediaType>, 0);
        }
        d3d_input::detach(&self.transform);
        // Events of the old stream give no credit for the new one.
        for _ in 0..64 {
            // SAFETY: a non-blocking poll of the transform's event queue.
            if unsafe { self.events.GetEvent(MF_EVENT_FLAG_NO_WAIT) }.is_err() {
                break;
            }
        }
        self.credits = 0;
        start(&self.transform, &self.setup, self.bitrate)?;
        self.sequence_header = mf_capture_output::sequence_header(&self.transform);
        Ok(())
    }
}

impl ScreenEncoder for AsyncEncoder {
    fn encode_captured(
        &mut self,
        image: &CapturedImage,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        self.encode_picture(&Picture::Cpu(image.clone()), request)
    }

    /// A GPU picture goes in as its ring texture, or (system memory input)
    /// as its NV12 copied from the slot; never through BGRA (BK3b).
    fn encode_picture(
        &mut self,
        image: &Picture,
        request: &StageRequest,
    ) -> io::Result<CapturedEncodedFrame> {
        let grant = self
            .grant
            .clone()
            .ok_or_else(|| io::Error::other("MF encoder has no capture grant"))?;
        let maximum = mf_capture_output::capture_limit(&grant, image, request, self.setup.size)?;
        ensure_com();
        let (limit, fps) = (self.limit, self.setup.fps);
        let (api, target, peak) = (self.api.as_ref(), request.target_bps, self.peak);
        if let Some(refused) = retune_bounded(api, &mut self.bitrate, target, limit, fps, peak) {
            self.rate_refused = refused;
            if refused {
                // B6: still above a lowered rate, the encoder gives way.
                refused_lowering(self.bitrate, target, limit)?;
            }
        }
        let key = request.keyframe || self.restarted;
        if key && (self.frames > 0 || self.restarted) {
            force_keyframe(self.api.as_ref());
        }
        if self.ltr && (key || self.frames == 0) {
            mark_ltr(self.api.as_ref());
        }
        match self.encode_once(&grant, image, request, maximum) {
            Ok(frame) => {
                self.refresh = false;
                self.restarted = false;
                Ok(frame)
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                self.refresh = true;
                Err(error)
            }
            // Fix of 2026-11-18: the capture changed its path and its grant
            // retired; turnover, never a GPU failure (it switched the GPU
            // path off for the session before). The pipeline opens the
            // encoder of the new grant.
            Err(error) if turnover(&grant, &error) => {
                self.refresh = true;
                Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!("the capture changed its path: {error}"),
                ))
            }
            Err(error) if self.gpu.is_some() => self.gpu_failed(error),
            Err(error) => Err(error),
        }
    }

    fn name(&self) -> String {
        match &self.gpu {
            Some(gpu) if gpu.is_own() => {
                format!("{} [GPU input (D3D11, own device)]", self.setup.name)
            }
            Some(_) => format!("{} [GPU input (D3D11)]", self.setup.name),
            None => self.setup.name.clone(),
        }
    }

    fn size(&self) -> FrameSize {
        self.setup.size
    }

    fn needs_refresh(&self) -> bool {
        self.refresh
    }

    fn rate_state(&self) -> Option<EncoderRateState> {
        Some(EncoderRateState {
            applied_bps: u64::from(self.bitrate),
            refused: self.rate_refused,
        })
    }

    /// Without admission (the run-time check, `bench --local`):
    /// `mf_encoder_plain.rs`.
    fn encode(
        &mut self,
        frame: &BgraImage,
        keyframe: bool,
        target_bps: u64,
    ) -> io::Result<EncodedFrame> {
        self.encode_plain(frame, keyframe, target_bps)
    }
}

/// Whether `error` of an encode under `grant` is the turnover of a capture
/// (its grant retired), not a failure of the encoder or its GPU path.
fn turnover(grant: &CaptureGrant, error: &io::Error) -> bool {
    CaptureGrant::is_retired_claim(error) || grant.is_retired().unwrap_or(false)
}

impl Drop for AsyncEncoder {
    fn drop(&mut self) {
        stop(&self.transform);
    }
}
