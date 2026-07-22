use std::io::{self, Read};
use std::sync::Arc;

use iroh::endpoint::{RecvStream, VarInt};

use super::backend::{recv_tagged, ShareIrohNode};
use super::core::eio;
use super::io_deadline;

const PEER_ABORT_CODE: VarInt = VarInt::from_u32(1);

pub(super) fn reader(
    node: Arc<ShareIrohNode>,
    recv: RecvStream,
    size: u64,
    data_tag: u8,
    session_key: String,
    generation: usize,
) -> Box<dyn Read + Send> {
    Box::new(PeerReader {
        node,
        recv,
        remaining: size,
        data_tag,
        buf: Vec::new(),
        pos: 0,
        terminal: None,
        session_key,
        generation,
    })
}

struct PeerReader {
    node: Arc<ShareIrohNode>,
    recv: RecvStream,
    remaining: u64,
    data_tag: u8,
    buf: Vec<u8>,
    pos: usize,
    terminal: Option<(io::ErrorKind, String)>,
    session_key: String,
    generation: usize,
}

impl Read for PeerReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if let Some((kind, message)) = &self.terminal {
            return Err(io::Error::new(*kind, message.clone()));
        }
        if self.remaining == 0 {
            return Ok(0);
        }
        while self.pos >= self.buf.len() {
            let frame = self.node.block_on(io_deadline::run(
                "peer read data",
                recv_tagged(&mut self.recv),
            ));
            let (tag, payload) = match frame {
                Ok(frame) => frame,
                Err(error) => return Err(self.close_with(error)),
            };
            if tag != self.data_tag {
                let error = eio("unerwarteter Frame beim Lesen");
                return Err(self.close_with(error));
            }
            if payload.len() as u64 > self.remaining {
                let error = eio("Peer sendet mehr Daten als angekuendigt");
                return Err(self.close_with(error));
            }
            self.buf = payload;
            self.pos = 0;
            if self.buf.is_empty() && self.remaining > 0 {
                continue;
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        self.remaining = self.remaining.saturating_sub(n as u64);
        Ok(n)
    }
}

impl PeerReader {
    fn close_with(&mut self, error: io::Error) -> io::Error {
        let _ = self.recv.stop(PEER_ABORT_CODE);
        let _ = self
            .node
            .invalidate_outgoing_session(&self.session_key, self.generation);
        self.buf.clear();
        self.pos = 0;
        self.terminal = Some((error.kind(), error.to_string()));
        error
    }
}
