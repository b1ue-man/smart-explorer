//! The hardware MFT without admission (the run-time check of plan
//! 2026-11-17 „Windows-Bild“ P3, `bench --local`): NV12 in system memory, or
//! on a device of its own the picture copied up (`GpuInput::sample_bgra`);
//! the output follows the key-frame rule D8 (`mf_capture_output::checked_unit`).
//! Split from `mf_encoder.rs`.
use std::io;
use std::time::Instant;

use super::super::mf_capture_output;
use super::super::mf_common::ensure_com;
use super::super::mf_encoder_sync::{force_keyframe, UNITS_PER_SECOND};
use super::super::mf_media::{
    input_sample, is_keyframe, nv12_bytes, process_output, retune_bounded, sample_bytes, Output,
};
use super::{AsyncEncoder, Event, EVENT_DEADLINE};
use crate::screen::core::host_backend::EncodedFrame;
use crate::screen::core::pixel_convert::BgraImage;

impl AsyncEncoder {
    pub(super) fn encode_plain(
        &mut self,
        frame: &BgraImage,
        keyframe: bool,
        target_bps: u64,
    ) -> io::Result<EncodedFrame> {
        if self.grant.is_some() {
            return Err(io::Error::other(
                "admitted MF encoder requires encode_captured",
            ));
        }
        if frame.size() != self.setup.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame size differs from the Media Foundation encoder",
            ));
        }
        ensure_com();
        let (limit, fps) = (self.limit, self.setup.fps);
        retune_bounded(self.api.as_ref(), &mut self.bitrate, target_bps, limit, fps);
        if keyframe && self.frames > 0 {
            force_keyframe(self.api.as_ref());
        }
        let deadline = Instant::now() + EVENT_DEADLINE;
        self.await_input(deadline)?;
        let duration = UNITS_PER_SECOND / i64::from(self.setup.fps.max(1));
        let time = self.frames * duration;
        // P3 (b): on a device of its own the picture is copied up.
        let sample = match self.gpu.as_mut() {
            Some(gpu) => gpu.sample_bgra(&frame.view(), time, duration)?,
            None => input_sample(&nv12_bytes(frame)?, time, duration)?,
        };
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
                        if let Some(gpu) = self.gpu.as_mut() {
                            gpu.finish();
                        }
                        // D8: the mark against the stream, parameter sets
                        // completed.
                        let frame = mf_capture_output::checked_unit(
                            self.setup.codec,
                            sample_bytes(&out)?,
                            is_keyframe(&out),
                            keyframe,
                            &mut self.parameter_sets,
                            self.sequence_header.as_deref(),
                        )?;
                        for data in &frame.records {
                            crate::screen::core::frame_size_caps::check_video_dimensions(
                                self.setup.codec,
                                data,
                                self.setup.size,
                                crate::screen::core::frame_budget::FrameLimits::RESOURCE,
                            )?;
                        }
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
