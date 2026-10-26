//! Hardware H.264 encoders of Media Foundation (asynchronous MFTs; SCR-M3,
//! decision C2): unlocked with `MF_TRANSFORM_ASYNC_UNLOCK` and driven by
//! their events (`METransformNeedInput`, `METransformHaveOutput`), which are
//! polled against a deadline, so a stuck driver ends in an error instead of
//! a hang. In low-latency mode each input gives its output before the next
//! input; an output that arrives without an input of this frame is an
//! encoder failure (a lost reference would corrupt the stream). Settings,
//! bit rate and key frames as the software encoder.
use std::io;
use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Win32::Media::MediaFoundation::{
    ICodecAPI, IMFMediaEventGenerator, IMFTransform, MEError, METransformHaveOutput,
    METransformNeedInput, MF_EVENT_FLAG_NO_WAIT, MF_E_NO_EVENTS_AVAILABLE,
    MF_TRANSFORM_ASYNC_UNLOCK,
};

use super::mf_common::{ensure_com, EncoderSetup};
use super::mf_encoder_sync::{
    clamp_bitrate, force_keyframe, frame_of, retune, start, stop, UNITS_PER_SECOND,
};
use super::mf_media::{
    configure_encoder, input_sample, is_keyframe, nv12_bytes, process_output, sample_bytes, Output,
};
use crate::screen::core::host_backend::{EncodedFrame, ScreenEncoder};
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::rate_control::START_DIRECT_BPS;

/// Longest wait for one event of the transform.
const EVENT_DEADLINE: Duration = Duration::from_secs(2);
/// Pause between two polls of the event queue.
const EVENT_POLL: Duration = Duration::from_millis(1);

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
}

// SAFETY: as `SyncEncoder`: one thread at a time, free-threaded objects of
// the multithreaded apartment.
unsafe impl Send for AsyncEncoder {}

impl AsyncEncoder {
    pub(super) fn new(transform: IMFTransform, setup: EncoderSetup) -> io::Result<Self> {
        // SAFETY: attribute write on the activated transform; without it an
        // asynchronous MFT refuses every call.
        unsafe {
            transform
                .GetAttributes()?
                .SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?
        };
        let events: IMFMediaEventGenerator = transform.cast()?;
        let bitrate = clamp_bitrate(START_DIRECT_BPS);
        let api = configure_encoder(&transform, bitrate);
        start(&transform, &setup, bitrate)?;
        Ok(Self {
            transform,
            events,
            api,
            setup,
            frames: 0,
            bitrate,
            credits: 0,
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
}

impl ScreenEncoder for AsyncEncoder {
    fn name(&self) -> String {
        self.setup.name.clone()
    }

    fn size(&self) -> FrameSize {
        self.setup.size
    }

    fn encode(
        &mut self,
        frame: &BgraImage,
        keyframe: bool,
        target_bps: u64,
    ) -> io::Result<EncodedFrame> {
        if frame.size() != self.setup.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame size differs from the Media Foundation encoder",
            ));
        }
        ensure_com();
        retune(self.api.as_ref(), &mut self.bitrate, target_bps);
        if keyframe && self.frames > 0 {
            force_keyframe(self.api.as_ref());
        }
        let deadline = Instant::now() + EVENT_DEADLINE;
        while self.credits == 0 {
            match self.next_event(deadline)? {
                Event::NeedInput => self.credits += 1,
                Event::HaveOutput => {
                    return Err(io::Error::other(
                        "the hardware encoder gave a picture without its frame",
                    ));
                }
                Event::Other => {}
            }
        }
        let duration = UNITS_PER_SECOND / i64::from(self.setup.fps.max(1));
        let sample = input_sample(&nv12_bytes(frame)?, self.frames * duration, duration)?;
        self.frames += 1;
        // SAFETY: one owned input sample for stream 0 after `NeedInput`.
        unsafe { self.transform.ProcessInput(0, &sample, 0)? };
        self.credits -= 1;
        let fallback = self.setup.size.bgra_len();
        loop {
            match self.next_event(deadline)? {
                Event::NeedInput => self.credits += 1,
                Event::HaveOutput => match process_output(&self.transform, fallback)? {
                    Output::Sample(out) => {
                        let key = is_keyframe(&out);
                        let frame=frame_of(vec![sample_bytes(&out)?],key,keyframe)?;
                        for data in &frame.records {crate::screen::core::frame_size_caps::check_avc_dimensions(data,self.setup.size,crate::screen::core::frame_budget::FrameLimits::RESOURCE)?;}
                        return Ok(frame);
                    }
                    Output::NeedInput => {}
                    Output::StreamChange => {
                        return Err(io::Error::other("the encoder changed its output type"));
                    }
                },
                Event::Other => {}
            }
        }
    }
}

impl Drop for AsyncEncoder {
    fn drop(&mut self) {
        stop(&self.transform);
    }
}
