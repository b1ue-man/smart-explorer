//! Opening admitted and raw codecs with the registry's established fallback order.
//! Plan 2026-11-17 „Windows-Bild“ P3: with checks an encoder without a
//! verdict runs its round trip first, and one that failed it is skipped like
//! a refusal; a decoder that failed its check is not opened.
use std::io;
use std::sync::Arc;

use super::super::host_backend::{ScreenDecoder, TilesScreenDecoder, TilesScreenEncoder};
use super::super::negotiate::{CodecChoice, CodecKind};
use super::super::pixel_convert::FrameSize;
use super::super::wire_binary::GopHeader;
use super::{encodes_stream, CodecRegistry, EncoderFactory, OpenedEncoder, TILES_NAME};

impl CodecRegistry {
    /// The gate of P3 (`Checks::admit`): `Err` with the reason skips
    /// `factory`; without checks every encoder opens, as before.
    fn admit(
        &self,
        factory: &Arc<dyn EncoderFactory>,
        choice: &CodecChoice,
        size: FrameSize,
    ) -> Result<(), String> {
        self.checks.as_ref().map_or(Ok(()), |checks| {
            checks.admit(factory, &self.decoders, choice, size)
        })
    }

    fn matching_encoders(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        refused: &[String],
        contract: Option<super::super::frame_budget::FrameContract>,
    ) -> Vec<(bool, &Arc<dyn EncoderFactory>)> {
        let mut seen: Vec<String> = Vec::new();
        let mut candidates: Vec<(bool, bool, &Arc<dyn EncoderFactory>)> = Vec::new();
        for factory in &self.encoders {
            let name = factory.name();
            if refused.contains(&name) || seen.contains(&name) {
                continue;
            }
            let best = factory
                .offers()
                .into_iter()
                .filter_map(|offer| {
                    let exact = encodes_stream(&offer, choice.codec, choice.profile)?;
                    offer
                        .supports_size_rate(size, fps, contract)
                        .then_some((exact, offer.hardware))
                })
                .min_by_key(|(exact, hardware)| (!*exact, *hardware != choice.encoder_hardware));
            if let Some((exact, hardware)) = best {
                seen.push(name);
                candidates.push((exact, hardware, factory));
            }
        }
        candidates
            .sort_by_key(|(exact, hardware, _)| (!*exact, *hardware != choice.encoder_hardware));
        candidates
            .into_iter()
            .map(|(_, hardware, factory)| (hardware, factory))
            .collect()
    }

    /// Number of distinct encoder stages available in this registry for
    /// `request` before session refusals (W4, B3).
    pub fn encoder_stage_count(&self, request: &super::super::host_stage::StageRequest) -> usize {
        if request.choice.codec == CodecKind::Tiles {
            return 1;
        }
        self.matching_encoders(
            &request.choice,
            request.size,
            request.fps,
            &[],
            request.frame_contract,
        )
        .len()
    }

    /// The worker and in-process capture use the same negotiated wrapper.
    pub fn open_encoder_for_stage(
        &self,
        request: &super::super::host_stage::StageRequest,
        refused: &[String],
    ) -> io::Result<OpenedEncoder> {
        if let Some(contract) = request.frame_contract {
            contract.validate()?;
        }
        request
            .frame_contract
            .map_or(super::super::frame_budget::FrameLimits::LEGACY, |c| {
                c.limits
            })
            .check(request.size)?;
        if request.size.pixels() > request.choice.max_pixels {
            return Err(super::super::wire::invalid(
                "frame exceeds codec pixel limit",
            ));
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
    pub fn open_encoder_for_stage_admitted(
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
            .map_or(super::super::frame_budget::FrameLimits::LEGACY, |c| {
                c.limits
            });
        limits.check(request.size)?;
        if request.size.pixels() > request.choice.max_pixels {
            return Err(super::super::wire::invalid(
                "frame exceeds codec pixel limit",
            ));
        }
        let mut opened = if request.choice.codec == CodecKind::Tiles {
            OpenedEncoder {
                encoder: Box::new(
                    super::super::host_codec_adapters::AdmittedTilesEncoder::new(
                        request.size,
                        limits,
                        grant.clone(),
                    )?,
                ),
                factory: TILES_NAME.into(),
                hardware: false,
            }
        } else {
            let candidates = self.matching_encoders(
                &request.choice,
                request.size,
                request.fps,
                refused,
                request.frame_contract,
            );
            let mut selected = None;
            let mut refusals = Vec::new();
            for (hardware, factory) in candidates {
                if let Err(reason) = self.admit(factory, &request.choice, request.size) {
                    refusals.push(format!("{}: {reason}", factory.name()));
                    continue;
                }
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
    pub fn open_encoder(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
    ) -> io::Result<OpenedEncoder> {
        self.open_encoder_except(choice, size, fps, &[])
    }

    /// `open_encoder` without the factories named in `refused` (a host
    /// process skips an encoder that failed while encoding).
    pub fn open_encoder_except(
        &self,
        choice: &CodecChoice,
        size: FrameSize,
        fps: u32,
        refused: &[String],
    ) -> io::Result<OpenedEncoder> {
        self.open_encoder_with_contract(choice, size, fps, refused, None)
    }

    pub fn open_encoder_with_contract(
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
        let limits = contract.map_or(super::super::frame_budget::FrameLimits::LEGACY, |c| {
            c.limits
        });
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
        let candidates = self.matching_encoders(choice, size, fps, refused, contract);
        let mut refusals: Vec<String> = Vec::new();
        for (hardware, factory) in candidates {
            if let Err(reason) = self.admit(factory, choice, size) {
                refusals.push(format!("{}: {reason}", factory.name()));
                continue;
            }
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
        let profile = choice.codec.profile_name(choice.profile);
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
    pub fn open_decoder(&self, header: &GopHeader) -> io::Result<Box<dyn ScreenDecoder>> {
        if let Some(contract) = header.frame_contract {
            contract.validate()?;
        }
        header
            .frame_contract
            .map_or(super::super::frame_budget::FrameLimits::LEGACY, |c| {
                c.limits
            })
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
        let guarded = if header.codec.is_video()
            && !header.frame_contract.is_some_and(|c| c.hybrid_regions)
        {
            super::super::frame_reference::ReferenceDecoder::new_for_codec(decoder, header.codec)
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
        // „Dekodierer“ of the viewer (4.10): only implementations with a
        // stage of the chosen kind; one of both kinds (Media Foundation)
        // picks its stage itself from the same choice.
        let preference = super::super::negotiate::decoder_preference();
        let mut refusals: Vec<String> = Vec::new();
        for factory in &self.decoders {
            let decodes = factory.offers().iter().any(|offer| {
                offer.decodes(header.codec, header.profile)
                    && preference.allows(offer.hardware)
                    && offer.supports_size_rate(header.size, 0, header.frame_contract)
            });
            if !decodes {
                continue;
            }
            if self.decoder_failed(factory.as_ref(), header.codec) {
                refusals.push(format!("{}: failed its run-time check", factory.name()));
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
                format!("no {} decoder for {}", preference.key(), header.codec.key())
            } else {
                format!("no decoder opened: {}", refusals.join("; "))
            },
        ))
    }
}
