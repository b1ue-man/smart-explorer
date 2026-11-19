//! Captured frame production and admitted codec replacement.
use super::*;
use crate::screen::core::capture_picture::SourcePicture;
use std::sync::atomic::AtomicU64;

/// Review 2026-11-19 (F3): the lowest bit rate the `PRODUCE` encoder runs
/// at above the rate asked (as `Encoding::report_floor` reports it for the
/// stream), and the value last sent; one host process serves one session.
/// The request loop sends a change ahead of its answer (`report_floor`).
static PRODUCE_FLOOR: AtomicU64 = AtomicU64::new(0);
static PRODUCE_FLOOR_SENT: AtomicU64 = AtomicU64::new(0);

/// Sends a changed `PRODUCE_FLOOR` as `ENCODER_FLOOR` (0: none again).
pub(super) fn report_floor(output: &dispatch::Output) -> io::Result<()> {
    let floor = PRODUCE_FLOOR.load(Ordering::Acquire);
    if PRODUCE_FLOOR_SENT.swap(floor, Ordering::AcqRel) == floor {
        return Ok(());
    }
    let mut output = output
        .lock()
        .map_err(|_| io::Error::other("worker output poisoned"))?;
    crate::screen::core::worker_wire::send_encoder_floor(&mut *output, floor)
}
impl Worker {
    pub(super) fn produce(&mut self, request: &StageRequest) -> WorkerReply {
        if let Err(error) = self.admit(request.frame_contract) {
            return failed(ScreenErrorCode::CaptureFailed, error);
        }
        if !self.capture_active {
            return self.source.as_ref().map_or_else(
                || failed(ScreenErrorCode::CaptureFailed, "screen source unavailable"),
                |source| {
                    WorkerReply::Frame(StageFrame::Unchanged {
                        source_size: source.size(),
                    })
                },
            );
        }
        self.crash_hook();
        self.check_openh264();
        let refresh = request.keyframe
            || self.encoder.as_ref().is_none_or(|running| {
                running.epoch != request.encoder_epoch
                    || !same_encoder_choice(&running.choice, &request.choice, request.size)
                    || running.frame_contract != request.frame_contract
                    || running.encoder.size() != request.size
                    || running.encoder.needs_refresh()
            });
        let Some(source) = self.source.as_mut() else {
            return failed(ScreenErrorCode::CaptureFailed, "no source is open");
        };
        let source_size = source.size();
        let limits = request
            .frame_contract
            .map_or(FrameLimits::LEGACY, |contract| contract.limits)
            .for_capture(source.frame_limits());
        if let Err(error) = limits.check(request.size) {
            return failed(ScreenErrorCode::CaptureFailed, error);
        }
        let before = source.reference_generation();
        if before != self.reference_generation {
            eprintln!(
                "screen capture: capture generation {} -> {before}: the encoder starts anew",
                self.reference_generation
            );
            self.reference_generation = before;
            self.encoder = None;
            return WorkerReply::Frame(StageFrame::Unchanged { source_size });
        }
        // A request made for an older, larger source waits for the next.
        if request.size.width > source_size.width || request.size.height > source_size.height {
            return WorkerReply::Frame(StageFrame::Unchanged { source_size });
        }
        let capture_ts_us = now_us();
        // A picture may stay on the GPU for a GPU encoder (BK3b).
        let captured = source.capture_picture(request.frame_id, refresh);
        let after = source.reference_generation();
        let source_size = source.size();
        if after != before {
            eprintln!(
                "screen capture: capture generation {} -> {after}: the encoder starts anew",
                self.reference_generation
            );
            self.reference_generation = after;
            self.encoder = None;
            return match captured {
                Ok(SourcePicture::Paused(state)) => WorkerReply::Frame(StageFrame::Paused(state)),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    WorkerReply::Frame(StageFrame::Unchanged { source_size })
                }
                Err(error) => capture_failed(&error),
                _ => WorkerReply::Frame(StageFrame::Unchanged { source_size }),
            };
        }
        let image = match captured {
            Ok(SourcePicture::Frame(image)) => image,
            Ok(SourcePicture::Unchanged) => {
                let source_size = source.size();
                return WorkerReply::Frame(StageFrame::Unchanged { source_size });
            }
            Ok(SourcePicture::Paused(state)) => {
                return WorkerReply::Frame(StageFrame::Paused(state))
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return WorkerReply::Frame(StageFrame::Unchanged { source_size })
            }
            Err(error) => return capture_failed(&error),
        };
        let source_size = image.size();
        if request.size.width > source_size.width || request.size.height > source_size.height {
            return WorkerReply::Frame(StageFrame::Unchanged { source_size });
        }
        let image = if image.size() == request.size {
            image
        } else {
            match image.scaled(
                request.size,
                limits,
                crate::screen::core::capture_admission::CaptureOwner::ScaledPixels,
            ) {
                Ok(image) => image,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    return WorkerReply::Frame(StageFrame::Unchanged { source_size })
                }
                Err(error) => return failed(ScreenErrorCode::CaptureFailed, error),
            }
        };
        // An encoder that fails while encoding gives way to every remaining
        // stage of the rank once (`refused`, W4 B3); the stages are counted
        // only after a failure (plan 2026-11-17 P3). A working instance that
        // fails first gets one fresh instance of its own factory
        // (`Running::failure_refuses`, review 2026-11-19 B1).
        let mut request = std::borrow::Cow::Borrowed(request);
        let mut max_attempts: Option<usize> = None;
        let mut attempt = 0;
        let mut fresh: Option<String> = None;
        let (frame, running) = loop {
            let running = match self.encoder_for(&request) {
                Ok(running) => running,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    return WorkerReply::Frame(StageFrame::Unchanged { source_size })
                }
                Err(error) => return failed(ScreenErrorCode::EncoderFailed, error),
            };
            running.replaces_failed |= fresh.take().as_ref() == Some(&running.factory);
            let encoded = running.encoder.encode_picture(&image, &request);
            match encoded.and_then(|frame| keyed(frame, &request)) {
                Ok(frame) => {
                    running.delivered += 1;
                    let floor = running
                        .encoder
                        .rate_state()
                        .filter(|state| !state.refused && state.applied_bps > request.target_bps)
                        .map_or(0, |state| state.applied_bps);
                    PRODUCE_FLOOR.store(floor, Ordering::Release);
                    break (frame, running);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    return WorkerReply::Frame(StageFrame::Unchanged { source_size })
                }
                Err(error) => {
                    let factory = running.factory.clone();
                    let refuses = running.failure_refuses() || gave_up(&error);
                    self.encoder = None;
                    if factory == crate::screen::core::codec_registry::TILES_NAME {
                        return failed(ScreenErrorCode::EncoderFailed, error);
                    }
                    request.to_mut().keyframe = true;
                    if !refuses {
                        eprintln!(
                            "screen encoder: {factory} failed: {error}: a fresh one with a key frame"
                        );
                        fresh = Some(factory);
                        continue;
                    }
                    attempt += 1;
                    if !self.refused.contains(&factory) {
                        self.refused.push(factory);
                    }
                    let stages = *max_attempts.get_or_insert_with(|| {
                        codec_inventory::registry()
                            .encoder_stage_count(&request)
                            .max(1)
                    });
                    if attempt >= stages {
                        return failed(ScreenErrorCode::EncoderFailed, error);
                    }
                }
            }
        };
        if frame.records.is_empty() {
            return WorkerReply::Frame(StageFrame::Unchanged { source_size });
        }
        if let Err(error) = crate::screen::core::frame_size_caps::validate_encoded_budget(
            request.frame_contract,
            request.choice.codec,
            &frame.records,
        ) {
            return failed(ScreenErrorCode::EncoderFailed, error);
        }
        let encoder = StageEncoder {
            name: running.encoder.name(),
            hardware: running.hardware,
        };
        WorkerReply::Frame(StageFrame::Encoded {
            records: frame.records,
            keyframe: frame.keyframe,
            capture_ts_us,
            encoder,
            source_size,
        })
    }

    /// The encoder for `request`: the running one while epoch, codec and
    /// size stay, else a new one from the codecs of this user (the old one
    /// is dropped first, so a library it held is unloaded before the next
    /// loads).
    fn encoder_for(&mut self, request: &StageRequest) -> io::Result<&mut Running> {
        let grant = self
            .source
            .as_ref()
            .and_then(|source| source.capture_grant())
            .ok_or_else(|| io::Error::other("worker source has no capture grant"))?;
        let book = grant.book_identity();
        let current = self.encoder.as_ref().is_some_and(|running| {
            running.book == book
                && running.epoch == request.encoder_epoch
                && same_encoder_choice(&running.choice, &request.choice, request.size)
                && running.frame_contract == request.frame_contract
                && running.encoder.size() == request.size
        });
        if !current {
            if let Some(running) = &self.encoder {
                eprintln!(
                    "screen encoder: opens anew ({})",
                    reopen_reason(running, request, book)
                );
            }
            self.encoder = None;
            let opened = codec_inventory::registry().open_encoder_for_stage_admitted(
                request,
                &self.refused,
                grant,
            )?;
            self.encoder = Some(Running {
                encoder: opened.encoder,
                factory: opened.factory,
                hardware: opened.hardware,
                epoch: request.encoder_epoch,
                choice: request.choice,
                frame_contract: request.frame_contract,
                book,
                delivered: 0,
                replaces_failed: false,
            });
        }
        self.encoder
            .as_mut()
            .ok_or_else(|| io::Error::other("no screen encoder"))
    }
}
