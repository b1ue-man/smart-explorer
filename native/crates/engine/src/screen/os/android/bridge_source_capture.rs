//! Epoch-bound Tiles and MediaCodec output for the existing Android source.
use super::*;

impl BridgeSource {
    pub(super) fn tiles_frame(&mut self, request: &StageRequest) -> Result<StageFrame, StageError> {
        let projected = matches!(
            self.channel.phase(),
            Phase::Ready {
                projection: true,
                ..
            }
        );
        if self.epoch != Some(request.encoder_epoch) {
            self.hybrid = None;
            self.tiles = None;
            if projected {
                self.channel.clear_pictures();
                self.channel.send(json!({"op":"configure","codec":"tiles","hybrid":true,
                    "epoch":request.encoder_epoch & 0xFFFF,"width":request.size.width,"height":request.size.height,
                    "fps":request.fps,"bitrate":request.target_bps,"profile":"high"}));
            }
            self.epoch = Some(request.encoder_epoch);
        }
        // Accessibility remains the preview/snapshot fallback when projection is absent.
        if !projected
            && !self.synthetic()
            && !super::super::bridge_channels::prereqs().accessibility_on()
        {
            return Ok(StageFrame::Paused(SessionState::NoDisplay));
        }
        let failed = |error| StageError::new(ScreenErrorCode::CaptureFailed, error);
        // Read native geometry before the single output downscale. Treating a
        // legacy-sized intermediate as source geometry would repeatedly report
        // a resize against the app's unchanged native display dimensions.
        let image = self
            .still(request.frame_id, FrameLimits::RESOURCE)
            .map_err(failed)?;
        let source_size = if projected { self.size } else { image.size() };
        if !projected && source_size != self.size {
            // Rotated or resized: the pipeline starts a new picture group.
            self.size = source_size;
            return Ok(StageFrame::Unchanged { source_size });
        }
        let size = request.size;
        let image = if image.size() == size {
            image
        } else {
            image
                .downscale(
                    size,
                    request
                        .frame_contract
                        .map_or(FrameLimits::LEGACY, |c| c.limits),
                    &image.grant(),
                    CaptureOwner::ScaledPixels,
                )
                .map_err(failed)?
        };
        let encoder_failed = |error| StageError::new(ScreenErrorCode::EncoderFailed, error);
        let generation = image.generation().value();
        let mut tiles = match self.tiles.take() {
            Some((epoch, previous, encoder))
                if epoch == request.encoder_epoch && previous == generation =>
            {
                encoder
            }
            _ => {
                let grant = image.grant();
                let encoder: Box<dyn ScreenEncoder> = Box::new(
                    crate::screen::core::host_codec_adapters::AdmittedTilesEncoder::new(
                        size,
                        request
                            .frame_contract
                            .map_or(FrameLimits::LEGACY, |c| c.limits),
                        grant.clone(),
                    )
                    .map_err(encoder_failed)?,
                );
                crate::screen::core::host_hybrid::wrap_captured(encoder, request, grant)
                    .map_err(encoder_failed)?
            }
        };
        let frame = tiles.encode_captured(&image, request);
        self.tiles = Some((request.encoder_epoch, generation, tiles));
        let frame = frame.map_err(encoder_failed)?;
        if frame.records.is_empty() {
            return Ok(StageFrame::Unchanged { source_size });
        }
        Ok(StageFrame::Encoded {
            records: frame.records,
            keyframe: frame.keyframe,
            capture_ts_us: now_us(),
            encoder: StageEncoder {
                name: "tiles".into(),
                hardware: false,
            },
            source_size,
        })
    }

    pub(super) fn h264_frame(
        &mut self,
        request: &StageRequest,
        encoder: StageEncoder,
    ) -> Result<StageFrame, StageError> {
        let source_size = self.size;
        let hybrid = request.frame_contract.is_some_and(|c| c.hybrid_regions) && !self.synthetic();
        let fresh = self.epoch != Some(request.encoder_epoch);
        if fresh {
            self.hybrid = None;
            self.tiles = None;
            self.channel.clear_pictures();
            self.channel.send(json!({
                "op": "configure",
                "epoch": request.encoder_epoch & 0xFFFF,
                "codec": "h264",
                "profile": request.choice.profile.map(|profile| profile.key()),
                "width": request.size.width,
                "height": request.size.height,
                "fps": request.fps,
                "bitrate": request.target_bps,
                "hybrid": hybrid,
            }));
            self.epoch = Some(request.encoder_epoch);
            self.bitrate = request.target_bps;
        } else {
            if request.keyframe {
                self.channel.send(json!({"op": "keyframe"}));
            }
            let step = self.bitrate * BITRATE_STEP_PERCENT / 100;
            if request.target_bps.abs_diff(self.bitrate) > step {
                self.channel
                    .send(json!({"op": "bitrate", "bps": request.target_bps}));
                self.bitrate = request.target_bps;
            }
        }
        if hybrid {
            self.channel
                .send(json!({"op":"capture","frameId":request.frame_id}));
        }
        let epoch = u32::try_from(request.encoder_epoch & 0xFFFF).unwrap_or(0);
        let want_key = fresh || request.keyframe;
        let timeout = if want_key || hybrid {
            KEY_TIMEOUT
        } else {
            frame_wait(request.fps)
        };
        let accept = |picture: &Picture| {
            !picture.still()
                && picture.epoch() == epoch
                && (!want_key || picture.keyframe())
                && (!hybrid
                    || (picture.captured_video()
                        && picture.data.as_slice().get(8..16).is_some_and(|bytes| {
                            u64::from_le_bytes(bytes.try_into().unwrap_or([0; 8]))
                                == request.frame_id
                        })))
        };
        let picture = self
            .channel
            .next_picture(timeout, accept)
            .map_err(ended_error)?;
        // Kotlin admits the first paired capture only after publishing the actual encoder.
        let encoder = match self.channel.phase() {
            Phase::Ready {
                width,
                height,
                encoder,
                hardware,
                ..
            } => {
                let reported = FrameSize::new(width, height);
                if width > 0 && height > 0 && reported != source_size {
                    self.size = reported;
                    self.reported = Some(reported);
                    return Ok(StageFrame::Unchanged {
                        source_size: reported,
                    });
                }
                StageEncoder {
                    name: encoder,
                    hardware,
                }
            }
            Phase::Ended(reason) => return Err(ended_error(reason)),
            _ => encoder,
        };
        match picture {
            Some(picture) if hybrid && picture.flags & 0x10 != 0 => {
                let fail = |error| StageError::new(ScreenErrorCode::EncoderFailed, error);
                let state = self.hybrid.as_mut().ok_or_else(|| {
                    fail(io::Error::other("unchanged Android capture without base"))
                })?;
                let frame = state.2.repeat_external(request).map_err(fail)?;
                if frame.records.is_empty() {
                    Ok(StageFrame::Unchanged { source_size })
                } else {
                    Ok(StageFrame::Encoded {
                        records: frame.records,
                        keyframe: frame.keyframe,
                        capture_ts_us: now_us(),
                        encoder,
                        source_size,
                    })
                }
            }
            Some(picture) if hybrid => {
                let fail = |error| StageError::new(ScreenErrorCode::EncoderFailed, error);
                let data = picture.data.as_slice();
                let grant = picture.data.grant().map_err(fail)?;
                let size = request.size;
                let limits = request
                    .frame_contract
                    .map_or(FrameLimits::LEGACY, |c| c.limits);
                let raw_len = limits.check(size).map_err(fail)?;
                let end = 20usize
                    .checked_add(raw_len)
                    .ok_or_else(|| fail(io::Error::other("Android frame size overflow")))?;
                let head = data
                    .get(..20)
                    .ok_or_else(|| fail(io::Error::other("Android paired frame without header")))?;
                let width = u32::from_le_bytes(
                    head[0..4]
                        .try_into()
                        .map_err(|_| fail(io::Error::other("Android frame width")))?,
                );
                let height = u32::from_le_bytes(
                    head[4..8]
                        .try_into()
                        .map_err(|_| fail(io::Error::other("Android frame height")))?,
                );
                let video_len = u32::from_le_bytes(
                    head[16..20]
                        .try_into()
                        .map_err(|_| fail(io::Error::other("Android frame video length")))?,
                ) as usize;
                if FrameSize::new(width, height) != size
                    || video_len > crate::screen::core::wire_binary::MAX_RECORD_BYTES
                    || end.checked_add(video_len) != Some(data.len())
                {
                    return Err(fail(io::Error::other(
                        "Android raw/video pair size mismatch",
                    )));
                }
                let mut pixels =
                    AdmittedBytes::copy(&grant, CaptureOwner::Readback, &data[20..end])
                        .map_err(fail)?;
                for pixel in pixels.as_mut_slice().chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                    pixel[3] = 255;
                }
                let image = CaptureImageBuilder::from_bytes(size, limits, &grant, pixels)
                    .map_err(fail)?
                    .freeze();
                let generation = grant.generation().value();
                let fresh = self.hybrid.as_ref().is_none_or(|(epoch, previous, _)| {
                    *epoch != request.encoder_epoch || *previous != generation
                });
                if fresh {
                    self.hybrid = Some((
                        request.encoder_epoch,
                        generation,
                        crate::screen::core::host_hybrid::CapturedExternalHybrid::new(
                            request,
                            grant.clone(),
                        )
                        .map_err(fail)?,
                    ));
                }
                let mut stamped = request.clone();
                stamped.keyframe |= picture.keyframe();
                let state = self
                    .hybrid
                    .as_mut()
                    .ok_or_else(|| fail(io::Error::other("missing Android hybrid encoder")))?;
                let records = if video_len == 0 {
                    CapturedRecords::empty(&grant)
                } else {
                    AdmittedBytes::copy(&grant, CaptureOwner::CodecOutput, &data[end..])
                        .and_then(CapturedRecords::one)
                }
                .map_err(fail)?;
                let frame = state
                    .2
                    .encode_external(
                        &image,
                        &stamped,
                        crate::screen::core::host_backend::CapturedEncodedFrame {
                            records,
                            keyframe: picture.keyframe(),
                        },
                    )
                    .map_err(fail)?;
                if frame.records.is_empty() {
                    return Ok(StageFrame::Unchanged { source_size });
                }
                Ok(StageFrame::Encoded {
                    records: frame.records,
                    keyframe: frame.keyframe,
                    capture_ts_us: now_us(),
                    encoder,
                    source_size,
                })
            }
            Some(picture) => {
                let keyframe = picture.keyframe();
                Ok(StageFrame::Encoded {
                    keyframe,
                    records: CapturedRecords::one(picture.data)
                        .map_err(|e| StageError::new(ScreenErrorCode::EncoderFailed, e))?,
                    capture_ts_us: now_us(),
                    encoder,
                    source_size,
                })
            }
            None if want_key => Err(StageError::new(
                ScreenErrorCode::EncoderFailed,
                io::Error::new(io::ErrorKind::TimedOut, "no key frame from MediaCodec"),
            )),
            None => Ok(StageFrame::Unchanged { source_size }),
        }
    }
}
