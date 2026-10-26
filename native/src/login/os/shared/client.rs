//! Boot-gateway IPC client. Every effect retains its peer signature.
use super::super::platform;
use crate::{
    login::core::{
        image_send,
        model::{Challenge, SignedRequest},
        wire::{self, Reply, Request, PIXELS},
    },
    screen::core::{
        capture_admission::{CaptureGeneration, SourceKey},
        capture_image::{CaptureImageBuilder, CapturedImage},
        frame_budget::FrameLimits,
        image_admission::{ImageAdmission, ImageRole},
        pixel_convert::FrameSize,
    },
};
use std::{io, sync::Arc};
pub(crate) struct GatewayClient {
    stream: platform::ClientStream,
    challenge: Challenge,
    images: Arc<ImageAdmission>,
    source: SourceKey,
}
impl GatewayClient {
    pub(crate) fn connect(peer_id: &str, target_user: &str) -> io::Result<Self> {
        let images = crate::screen::os::image_memory::admit_images(image_send::limits()?)?;
        let source = SourceKey::new(&super::nonce()?, "login:broker")?;
        let mut stream = platform::connect()?;
        wire::send(
            &mut stream,
            &Request::Hello {
                peer_id: peer_id.into(),
                target_user: target_user.into(),
            },
        )?;
        let reply: Reply = wire::receive(&mut stream)?;
        if reply.pixel_bytes != 0 {
            return Err(io::Error::other(
                "login broker startup cannot contain pixels",
            ));
        }
        let challenge = reply.challenge.ok_or_else(|| {
            io::Error::other(
                reply
                    .reason
                    .unwrap_or_else(|| "login broker returned no challenge".into()),
            )
        })?;
        Ok(Self {
            stream,
            challenge,
            images,
            source,
        })
    }
    pub(crate) fn challenge(&self) -> &Challenge {
        &self.challenge
    }
    pub(crate) fn images(&self) -> Arc<ImageAdmission> {
        self.images.clone()
    }
    pub(crate) fn send(
        &mut self,
        request: SignedRequest,
    ) -> io::Result<(Reply, Option<CapturedImage>)> {
        if request.challenge != self.challenge {
            return Err(io::Error::other(
                "request does not use this broker challenge",
            ));
        }
        let generation = CaptureGeneration::new(request.challenge.sequence);
        wire::send(&mut self.stream, &Request::Signed(request))?;
        let reply: Reply = wire::receive(&mut self.stream)?;
        let image = if reply.pixel_bytes == 0 {
            if reply.state == "pixels" {
                return Err(io::Error::other("empty broker pixel reply"));
            }
            None
        } else {
            let size = FrameSize::new(reply.width, reply.height);
            size.check_frame()?;
            if reply.state != "pixels" || size.checked_bgra_len()? != reply.pixel_bytes {
                return Err(io::Error::other(
                    "login broker frame length/geometry mismatch",
                ));
            }
            let (tag, length) = wire::read_header(&mut self.stream)?;
            if tag != PIXELS || length != reply.pixel_bytes {
                return Err(io::Error::other("login broker pixel header mismatch"));
            }
            let mut image = CaptureImageBuilder::new_local(
                size,
                FrameLimits::LEGACY,
                &self.images,
                ImageRole::DecodedInput,
                self.source.clone(),
                generation,
            )?;
            std::io::Read::read_exact(&mut self.stream, image.pixels_mut())?;
            Some(image.freeze())
        };
        if let Some(challenge) = &reply.challenge {
            self.challenge = challenge.clone();
        }
        Ok((reply, image))
    }
}
