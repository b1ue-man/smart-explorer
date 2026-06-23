use std::io::{self, Read, Write};
use std::net::TcpStream;

use super::core::{eio, hex, public_fingerprint};

const NOISE_PARAMS: &str = "Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s";

pub(crate) const TAG_CTRL: u8 = 0;
pub(crate) const TAG_DATA: u8 = 1;

pub(crate) struct Channel {
    t: snow::TransportState,
    s: TcpStream,
    remote_static: Vec<u8>,
}

impl Channel {
    pub(crate) fn initiator(
        mut s: TcpStream,
        psk: &[u8; 32],
        local_private: &[u8],
        expected_remote_public: Option<&[u8]>,
    ) -> io::Result<Channel> {
        let params = NOISE_PARAMS.parse().map_err(eio)?;
        let mut hs = snow::Builder::new(params)
            .local_private_key(local_private)
            .psk(3, psk)
            .build_initiator()
            .map_err(eio)?;

        let mut buf = vec![0u8; 1024];
        let n = hs.write_message(&[], &mut buf).map_err(eio)?;
        write_raw_frame(&mut s, &buf[..n])?;

        let msg = read_raw_frame(&mut s)?;
        let mut out = vec![0u8; 1024];
        hs.read_message(&msg, &mut out).map_err(eio)?;

        let n = hs.write_message(&[], &mut buf).map_err(eio)?;
        write_raw_frame(&mut s, &buf[..n])?;

        let remote_static = hs
            .get_remote_static()
            .ok_or_else(|| eio("Peer sendet keine statische Identitaet"))?
            .to_vec();
        verify_remote_static(&remote_static, expected_remote_public)?;
        let t = hs.into_transport_mode().map_err(eio)?;
        Ok(Channel {
            t,
            s,
            remote_static,
        })
    }

    pub(crate) fn responder(
        mut s: TcpStream,
        psk: &[u8; 32],
        local_private: &[u8],
        expected_remote_public: Option<&[u8]>,
    ) -> io::Result<Channel> {
        let params = NOISE_PARAMS.parse().map_err(eio)?;
        let mut hs = snow::Builder::new(params)
            .local_private_key(local_private)
            .psk(3, psk)
            .build_responder()
            .map_err(eio)?;

        let msg = read_raw_frame(&mut s)?;
        let mut out = vec![0u8; 1024];
        hs.read_message(&msg, &mut out).map_err(eio)?;

        let mut buf = vec![0u8; 1024];
        let n = hs.write_message(&[], &mut buf).map_err(eio)?;
        write_raw_frame(&mut s, &buf[..n])?;

        let msg = read_raw_frame(&mut s)?;
        hs.read_message(&msg, &mut out).map_err(eio)?;

        let remote_static = hs
            .get_remote_static()
            .ok_or_else(|| eio("Peer sendet keine statische Identitaet"))?
            .to_vec();
        verify_remote_static(&remote_static, expected_remote_public)?;
        let t = hs.into_transport_mode().map_err(eio)?;
        Ok(Channel {
            t,
            s,
            remote_static,
        })
    }

    pub(crate) fn remote_static(&self) -> &[u8] {
        &self.remote_static
    }

    pub(crate) fn remote_fingerprint(&self) -> String {
        public_fingerprint(&self.remote_static)
    }

    pub(crate) fn send(&mut self, tag: u8, payload: &[u8]) -> io::Result<()> {
        let mut plain = Vec::with_capacity(payload.len() + 1);
        plain.push(tag);
        plain.extend_from_slice(payload);
        let mut buf = vec![0u8; plain.len() + 32];
        let n = self.t.write_message(&plain, &mut buf).map_err(eio)?;
        write_raw_frame(&mut self.s, &buf[..n])
    }

    pub(crate) fn recv(&mut self) -> io::Result<(u8, Vec<u8>)> {
        let cipher = read_raw_frame(&mut self.s)?;
        let mut out = vec![0u8; cipher.len()];
        let n = self.t.read_message(&cipher, &mut out).map_err(eio)?;
        out.truncate(n);
        if out.is_empty() {
            return Err(eio("leerer Frame"));
        }
        let tag = out[0];
        Ok((tag, out[1..].to_vec()))
    }
}

fn verify_remote_static(remote: &[u8], expected: Option<&[u8]>) -> io::Result<()> {
    if let Some(expected) = expected {
        if remote != expected {
            return Err(eio(format!(
                "Identitaetskonflikt: erwartet {}, bekommen {}",
                hex(expected),
                hex(remote)
            )));
        }
    }
    Ok(())
}

pub(crate) fn write_raw_frame(s: &mut TcpStream, data: &[u8]) -> io::Result<()> {
    if data.len() > 70_000 {
        return Err(eio("Frame zu gross"));
    }
    s.write_all(&(data.len() as u32).to_be_bytes())?;
    s.write_all(data)?;
    s.flush()
}

pub(crate) fn read_raw_frame(s: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut len4 = [0u8; 4];
    s.read_exact(&mut len4)?;
    let n = u32::from_be_bytes(len4) as usize;
    if n > 70_000 {
        return Err(eio("Frame zu gross"));
    }
    let mut buf = vec![0u8; n];
    s.read_exact(&mut buf)?;
    Ok(buf)
}
