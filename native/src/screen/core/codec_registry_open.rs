//! Opening admitted and raw codecs with the registry's established fallback order.
use std::io;
use std::sync::Arc;

use super::super::host_backend::{ScreenDecoder, TilesScreenDecoder, TilesScreenEncoder};
use super::super::negotiate::{CodecChoice, CodecKind, H264Profile};
use super::super::pixel_convert::FrameSize;
use super::super::wire_binary::GopHeader;
use super::{offers_stream, CodecRegistry, EncoderFactory, OpenedEncoder, TILES_NAME};

impl CodecRegistry {
    /// The worker and in-process capture use the same negotiated wrapper.
    pub(crate) fn open_encoder_for_stage(
        &self,
        request: &super::super::host_stage::StageRequest,
        refused: &[String],
    ) -> io::Result<OpenedEncoder> {
        if let Some(contract) = request.frame_contract {
            contract.validate()?;
        }
        request
            .frame_contract
            .map_or(super::super::frame_budget::FrameLimits::LEGACY, |c| c.limits)
            .check(request.size)?;
        if request.size.pixels() > request.choice.max_pixels {
            return Err(super::super::wire::invalid("frame exceeds codec pixel limit"));
        }
        let mut opened = if let Some(contract) = request
            .frame_contract
            .filter(|_| request.choice.codec == CodecKind::Tiles)
        {
            OpenedEncoder {
                encoder: Box::new(TilesScreenEncoder::new_with_limits(
                    request.size,
                    contract.limits,
                )?),
                factory: TILES_NAME.into(),
                hardware: false,
            }
        } else {
            self.open_encoder_with_contract(
                &request.choice,
                request.size,
                request.fps,
                refused,
                request.frame_contract,
            )?
        };
        opened.encoder = super::super::host_hybrid::wrap(opened.encoder, request)?;
        Ok(opened)
    }
    pub(crate) fn open_encoder_for_stage_admitted(
        &self,
        request: &super::super::host_stage::StageRequest,
        refused: &[String],
        grant: super::super::capture_admission::CaptureGrant,
    ) -> io::Result<OpenedEncoder> {
        if let Some(contract) = request.frame_contract {
            contract.validate()?;
        }
        let limits = request
            .frame_contract
            .map_or(super::super::frame_budget::FrameLimits::LEGACY, |c| c.limits);
        limits.check(request.size)?;
        if request.size.pixels() > request.choice.max_pixels {
            return Err(super::super::wire::invalid("frame exceeds codec pixel limit"));
        }
        let mut opened = if request.choice.codec == CodecKind::Tiles {
            OpenedEncoder {
                encoder: Box::new(super::super::host_codec_adapters::AdmittedTilesEncoder::new(
                    request.size,
                    limits,
                    grant.clone(),
                )?),
                factory: TILES_NAME.into(),
                hardware: false,
            }
        } else {
            let mut candidates: Vec<_> = self
                .encoders
                .iter()
                .filter_map(|factory| {
                    if refused.contains(&factory.name()) {
                        return None;
                    }
                    factory
                        .offers()
                        .into_iter()
                        .find(|offer| {
                            offers_stream(offer, request.choice.codec, request.choice.profile)
                                && offer.supports_size_rate(
                                    request.size,
                                    request.fps,
                                    request.frame_contract,
                                )
                        })
                        .map(|offer| (offer.hardware, factory))
                })
                .collect();
            candidates.sort_by_key(|(hardware, _)| *hardware != request.choice.encoder_hardware);
            let mut selected = None;
            let mut refusals = Vec::new();
            for (hardware, factory) in candidates {
                match factory.open_admitted(
                    &request.choice,
                    request.size,
                    request.fps,
                    grant.clone(),
                ) {
                    Ok(encoder) => {
                        selected = Some(OpenedEncoder {
                            encoder,
                            factory: factory.name(),
                            hardware,
                        });
                        break;
                    }
                    Err(error) => refusals.push(format!("{}: {error}", factory.name())),
                }
            }
            selected.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!(
                        "no admitted {} encoder opened: {}",
                        request.choice.codec.key(),
                        refusals.join("; ")
                    ),
                )
            })?
        };
        opened.encoder = super::super::host_hybrid::wrap_captured(opened.encoder, request, grant)?;
        Ok(opened)
    }

    /// Opens an encoder for `choice` at `size`. H.264 tries every factory
    /// that offers the codec and profile within its largest frame, those with
    /// the chosen hardware flag first; the error names every refusal.
    pub(crate) fn open_encoder(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
    ) -> io::Result<OpenedEncoder> {
        self.open_encoder_except(choice, size, fps, &[])
    }

    /// `open_encoder` without the factories named in `refused` (a host
    /// process skips an encoder that failed while encoding).
    pub(crate) fn open_encoder_except(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        refused: &[String],
    ) -> io::Result<OpenedEncoder> {
        self.open_encoder_with_contract(choice, size, fps, refused, None)
    }

    pub(crate) fn open_encoder_with_contract(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        refused: &[String],
        contract: Option<super::super::frame_budget::FrameContract>,
    ) -> io::Result<OpenedEncoder> {
        if let Some(contract) = contract {
            contract.validate()?;
        }
        let limits = contract.map_or(super::super::frame_budget::FrameLimits::LEGACY, |c| c.limits);
        limits.check(size)?;
        if size.pixels() > choice.max_pixels {
            return Err(super::super::wire::invalid(
                "frame exceeds selected codec pixel limit",
            ));
        }
        if choice.codec == CodecKind::Tiles {
            return Ok(OpenedEncoder {
                encoder: Box::new(TilesScreenEncoder::new_with_limits(size, limits)?),
                factory: TILES_NAME.into(),
                hardware: false,
            });
        }
        let mut candidates: Vec<(bool, &Arc<dyn EncoderFactory>)> = Vec::new();
        for factory in &self.encoders {
            if refused.contains(&factory.name()) {
                continue;
            }
            let fitting = factory.offers().into_iter().find(|offer| {
                offers_stream(offer, choice.codec, choice.profile)
                    && offer.supports_size_rate(size, fps, contract)
            });
            if let Some(offer) = fitting {
                candidates.push((offer.hardware, factory));
            }
        }
        candidates.sort_by_key(|(hardware, _)| *hardware != choice.encoder_hardware);
        let mut refusals: Vec<String> = Vec::new();
        for (hardware, factory) in candidates {
            match factory.open(choice, size, fps) {
                Ok(encoder) => {
                    let factory = factory.name();
                    return Ok(OpenedEncoder {
                        encoder,
                        factory,
                        hardware,
                    });
                }
                Err(error) => refusals.push(format!("{}: {error}", factory.name())),
            }
        }
        let codec = choice.codec.key();
        let profile = choice.profile.map_or("", H264Profile::key);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            if refusals.is_empty() {
                format!(
                    "no {codec} {profile} encoder for {}x{}",
                    size.width, size.height
                )
            } else {
                format!(
                    "no {codec} {profile} encoder opened: {}",
                    refusals.join("; ")
                )
            },
        ))
    }

    /// Opens a decoder for the picture group of `header`: `tiles` directly,
    /// H.264 with the first factory whose profile decodes the stream's.
    pub(crate) fn open_decoder(&self, header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        if let Some(contract) = header.frame_contract {
            contract.validate()?;
        }
        header
            .frame_contract
            .map_or(super::super::frame_budget::FrameLimits::LEGACY, |c| c.limits)
            .check(header.size)?;
        let raw = self.open_decoder_raw(header)?;
        let decoder: Box<dyn ScreenDecoder> =
            if let Some(contract) = header.frame_contract.filter(|c| c.hybrid_regions) {
                Box::new(super::super::viewer_reconstruct::HybridDecoder::new(
                    raw,
                    header.size,
                    contract,
                )?)
            } else {
                raw
            };
        let guarded = if header.codec == CodecKind::H264
            && !header.frame_contract.is_some_and(|c| c.hybrid_regions)
        {
            super::super::frame_reference::ReferenceDecoder::new_video(decoder)
        } else {
            super::super::frame_reference::ReferenceDecoder::new(decoder)
        };
        Ok(Box::new(guarded))
    }

    fn open_decoder_raw(&self, header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        if header.codec == CodecKind::Tiles {
            return Ok(Box::new(if let Some(contract) = header.frame_contract {
                TilesScreenDecoder::new_with_limits(header.size, contract.limits)?
            } else {
                TilesScreenDecoder::new(header.size)?
            }));
        }
        let mut refusals: Vec<String> = Vec::new();
        for factory in &self.decoders {
            let decodes = factory.offers().iter().any(|offer| {
                offer.codec == header.codec
                    && match (offer.profile, header.profile) {
                        (Some(decoder), Some(stream)) => decoder.decodes(stream),
                        _ => false,
                    }
                    && offer.supports_size_rate(header.size, 0, header.frame_contract)
            });
            if !decodes {
                continue;
            }
            match factory.open(header) {
                Ok(decoder) => return Ok(decoder),
                Err(error) => refusals.push(format!("{}: {error}", factory.name())),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            if refusals.is_empty() {
                format!("no decoder for {}", header.codec.key())
            } else {
                format!("no decoder opened: {}", refusals.join("; "))
            },
        ))
    }

}
