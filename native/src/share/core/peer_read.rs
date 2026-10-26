use std::io::{self, Read};
use std::sync::Arc;

use iroh::endpoint::{RecvStream, VarInt};

use super::backend::ShareIrohNode;
use super::core::eio;
use super::framing::lz4_stream::{classify, Content};
use super::framing::{recv_content_frame, TAG_DATA_LZ4};
use super::io_deadline;
use super::quic_stats_hub::track_client;
use super::transfer_meter as stats;

const PEER_ABORT_CODE: VarInt = VarInt::from_u32(1);

pub(super) fn reader(
    node: Arc<ShareIrohNode>,
    recv: RecvStream,
    size: u64,
    data_tag: u8,
    lz4: bool,
    session_key: String,
    generation: usize,
) -> Box<dyn Read + Send> {
    track_client(&node, &session_key);
    stats::stream_started();
    Box::new(PeerReader {
        node,
        recv,
        remaining: size,
        data_tag,
        lz4,
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
    /// The request asked for LZ4 blocks, so `TAG_DATA_LZ4` frames are valid.
    lz4: bool,
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
            // Waiting for the next frame is waiting for the network.
            let waiting = stats::Timer::start();
            let frame = self.node.block_on_route(self.generation, io_deadline::run(
                "peer read data",
                recv_content_frame(&mut self.recv, self.lz4),
            ));
            waiting.stop(stats::Phase::NetWait);
            let (tag, payload) = match frame {
                Ok(frame) => frame,
                Err(error) => {
                    let error = if error.kind() == io::ErrorKind::UnexpectedEof {
                        // The host ended its stream before the announced
                        // length: the file shrank while it was read.
                        io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            format!("Quelle wurde während der Übertragung geändert ({error})"),
                        )
                    } else {
                        error
                    };
                    return Err(self.close_with(error));
                }
            };
            let payload = match classify(tag, payload, self.lz4) {
                Ok(Content::Bytes(bytes)) if tag == self.data_tag || tag == TAG_DATA_LZ4 => bytes,
                Ok(_) => {
                    let error = eio("unerwarteter Frame beim Lesen");
                    return Err(self.close_with(error));
                }
                Err(error) => return Err(self.close_with(error)),
            };
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
