//! Boot-gateway IPC client. The remote peer supplies every operation signature.
use std::io;
use crate::login::core::{model::{Challenge, SignedRequest}, wire::{self, Reply, Request, PIXELS}};
use super::super::platform;
pub(crate) struct GatewayClient { stream: platform::ClientStream, challenge: Challenge }
impl GatewayClient {
    pub(crate) fn connect(peer_id: &str, target_user: &str) -> io::Result<Self> {
        let mut stream = platform::connect()?;
        wire::send(&mut stream, &Request::Hello { peer_id: peer_id.into(), target_user: target_user.into() })?;
        let reply: Reply = wire::receive(&mut stream)?;
        let challenge = reply.challenge.ok_or_else(|| io::Error::other(reply.reason.unwrap_or_else(|| "login broker returned no challenge".into())))?;
        Ok(Self { stream, challenge })
    }
    pub(crate) fn challenge(&self) -> &Challenge { &self.challenge }
    pub(crate) fn send(&mut self, request: SignedRequest) -> io::Result<(Reply, Vec<u8>)> {
        if request.challenge != self.challenge { return Err(io::Error::other("request does not use this broker challenge")); }
        wire::send(&mut self.stream, &Request::Signed(request))?;
        let reply: Reply = wire::receive(&mut self.stream)?;
        let pixels = if reply.pixel_bytes == 0 { Vec::new() } else { wire::read_frame(&mut self.stream, PIXELS)? };
        if pixels.len() != reply.pixel_bytes { return Err(io::Error::other("login frame length mismatch")); }
        if let Some(challenge) = &reply.challenge { self.challenge = challenge.clone(); }
        Ok((reply, pixels))
    }
}
