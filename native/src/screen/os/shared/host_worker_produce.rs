//! Captured frame production and admitted codec replacement.
use super::*;
impl Worker {
    pub(super) fn produce(&mut self, request: &StageRequest) -> WorkerReply {
        if let Err(error)=self.admit(request.frame_contract) {return failed(ScreenErrorCode::CaptureFailed,error);}
        if !self.capture_active { return self.source.as_ref().map_or_else(||failed(ScreenErrorCode::CaptureFailed,"screen source unavailable"),|source|WorkerReply::Frame(StageFrame::Unchanged {source_size:source.size()})); }
        self.crash_hook();
        self.check_openh264();
        let refresh = request.keyframe
            || self.encoder.as_ref().is_none_or(|running| {
                running.epoch != request.encoder_epoch
                    || running.choice != request.choice
                    || running.frame_contract != request.frame_contract
                    || running.encoder.size() != request.size
                    || running.encoder.needs_refresh()
            });
        let Some(source) = self.source.as_mut() else {
            return failed(ScreenErrorCode::CaptureFailed, "no source is open");
        };
        let source_size = source.size();
        let limits=request.frame_contract.map_or(FrameLimits::LEGACY,|contract|contract.limits)
            .for_capture(source.frame_limits());
        if let Err(error)=limits.check(request.size) {return failed(ScreenErrorCode::CaptureFailed,error);}
        let before=source.reference_generation();
        if before!=self.reference_generation {
            self.reference_generation=before;self.encoder=None;
            return WorkerReply::Frame(StageFrame::Unchanged {source_size});
        }
        // A request made for an older, larger source waits for the next.
        if request.size.width > source_size.width || request.size.height > source_size.height {
            return WorkerReply::Frame(StageFrame::Unchanged { source_size });
        }
        let capture_ts_us = now_us();
        let captured=source.capture(request.frame_id,refresh);
        let after=source.reference_generation();let source_size=source.size();
        if after!=before {
            self.reference_generation=after;self.encoder=None;
            return match captured {
                Ok(SourceFrame::Paused(state))=>WorkerReply::Frame(StageFrame::Paused(state)),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock =>
                    WorkerReply::Frame(StageFrame::Unchanged {source_size}),
                Err(error)=>capture_failed(&error),
                _=>WorkerReply::Frame(StageFrame::Unchanged {source_size}),
            };
        }
        let image = match captured {
            Ok(SourceFrame::Frame(image)) => image,
            Ok(SourceFrame::Unchanged) => {
                let source_size = source.size();
                return WorkerReply::Frame(StageFrame::Unchanged { source_size });
            }
            Ok(SourceFrame::Paused(state)) => return WorkerReply::Frame(StageFrame::Paused(state)),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock =>
                return WorkerReply::Frame(StageFrame::Unchanged {source_size}),
            Err(error) => return capture_failed(&error),
        };
        let source_size = image.size();
        if request.size.width > source_size.width || request.size.height > source_size.height {
            return WorkerReply::Frame(StageFrame::Unchanged { source_size });
        }
        let image = if image.size() == request.size {
            image
        } else {
            match image.downscale(request.size,limits,&image.grant(),crate::screen::core::capture_admission::CaptureOwner::ScaledPixels) {
                Ok(image) => image,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock =>
                    return WorkerReply::Frame(StageFrame::Unchanged {source_size}),
                Err(error) => return failed(ScreenErrorCode::CaptureFailed, error),
            }
        };
        // An encoder that fails while encoding gives way to the next one of
        // the same codec (a hardware MFT to the software MFT, C2) once.
        let mut attempt = 0;
        let (frame, running) = loop {
            attempt += 1;
            let running = match self.encoder_for(request) {
                Ok(running) => running,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock =>
                    return WorkerReply::Frame(StageFrame::Unchanged {source_size}),
                Err(error) => return failed(ScreenErrorCode::EncoderFailed, error),
            };
            match running
                .encoder
                .encode_captured(&image, request)
            {
                Ok(frame) => break (frame, running),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock =>
                    return WorkerReply::Frame(StageFrame::Unchanged {source_size}),
                Err(error) => {
                    let factory = running.factory.clone();
                    self.encoder = None;
                    if attempt > 1 || factory == crate::screen::core::codec_registry::TILES_NAME {
                        return failed(ScreenErrorCode::EncoderFailed, error);
                    }
                    self.refused.push(factory);
                }
            }
        };
        if frame.records.is_empty() {
            return WorkerReply::Frame(StageFrame::Unchanged { source_size });
        }
        if let Err(error) = crate::screen::core::frame_size_caps::validate_encoded_budget(
            request.frame_contract, request.choice.codec, &frame.records,
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
        let grant = self.source.as_ref().and_then(|source| source.capture_grant())
            .ok_or_else(|| io::Error::other("worker source has no capture grant"))?;
        let book = grant.book_identity();
        let current = self.encoder.as_ref().is_some_and(|running| {
            running.book == book &&
            running.epoch == request.encoder_epoch
                && running.choice == request.choice
                && running.frame_contract == request.frame_contract
                && running.encoder.size() == request.size
        });
        if !current {
            self.encoder = None;
            let opened = codec_inventory::registry()
                .open_encoder_for_stage_admitted(request, &self.refused, grant)?;
            self.encoder = Some(Running {
                encoder: opened.encoder,
                factory: opened.factory,
                hardware: opened.hardware,
                epoch: request.encoder_epoch,
                choice: request.choice,
                frame_contract: request.frame_contract,
                book,
            });
        }
        self.encoder
            .as_mut()
            .ok_or_else(|| io::Error::other("no screen encoder"))
    }

}
