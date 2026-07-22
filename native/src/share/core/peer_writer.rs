use std::io::{self, Write};
use std::sync::Arc;

use iroh::endpoint::{RecvStream, SendStream};

use super::core::eio;
use super::framing::{decode_resp, recv_resp_wire, send_ctrl, send_tagged, TAG_DATA};
use super::fs;
use super::node::ShareIrohNode;
use super::wire::{Ctrl, FsRequest, FsResponse};

pub(super) fn writer(
    node: Arc<ShareIrohNode>,
    send: SendStream,
    recv: RecvStream,
    lease: Option<String>,
    session_key: String,
    generation: usize,
) -> Box<dyn Write + Send> {
    Box::new(PeerWriter {
        node,
        send: Some(send),
        recv: Some(recv),
        lease,
        session_key,
        generation,
        state: WriterState::Open,
    })
}

enum WriterState {
    Open,
    Committed,
    Failed(io::ErrorKind, String),
}

struct PeerWriter {
    node: Arc<ShareIrohNode>,
    send: Option<SendStream>,
    recv: Option<RecvStream>,
    lease: Option<String>,
    session_key: String,
    generation: usize,
    state: WriterState,
}

impl PeerWriter {
    fn finish(&mut self) -> io::Result<()> {
        match &self.state {
            WriterState::Committed => return Ok(()),
            WriterState::Failed(kind, message) => {
                return Err(io::Error::new(*kind, message.clone()))
            }
            WriterState::Open => {}
        }
        let Some(mut send) = self.send.take() else {
            return Err(eio("Peer-Schreibkanal geschlossen"));
        };
        let Some(mut recv) = self.recv.take() else {
            return Err(eio("Peer-Schreibantwort geschlossen"));
        };
        let result = self
            .node
            .block_on(super::io_deadline::run("peer write finish", async {
                send_ctrl(
                    &mut send,
                    &Ctrl::Fs {
                        req: FsRequest::WriteDone,
                        lease: self.lease.clone(),
                    },
                )
                .await?;
                recv_resp_wire(&mut recv).await
            }));
        match result {
            Ok(response) => match decode_resp(response) {
                Ok(FsResponse::Ok) => match send.finish().map_err(eio) {
                    Ok(()) => {
                        self.state = WriterState::Committed;
                        Ok(())
                    }
                    Err(error) => Err(self.fail_stream_parts(&mut send, &mut recv, error)),
                },
                Ok(_) => Err(self.fail_stream_parts(
                    &mut send,
                    &mut recv,
                    eio("unerwartete Antwort auf Schreib-Ende"),
                )),
                Err(error) => Err(self.remember_failure(error)),
            },
            Err(error) => Err(self.fail_stream_parts(&mut send, &mut recv, error)),
        }
    }

    fn remember_failure(&mut self, error: io::Error) -> io::Error {
        self.state = WriterState::Failed(error.kind(), error.to_string());
        error
    }

    fn fail_stream_parts(
        &mut self,
        send: &mut SendStream,
        recv: &mut RecvStream,
        error: io::Error,
    ) -> io::Error {
        super::io_deadline::abort(send, recv);
        let _ = self
            .node
            .invalidate_outgoing_session(&self.session_key, self.generation);
        self.state = WriterState::Failed(error.kind(), error.to_string());
        error
    }

    fn fail_open_stream(&mut self, error: io::Error) -> io::Error {
        if let (Some(send), Some(recv)) = (self.send.as_mut(), self.recv.as_mut()) {
            super::io_deadline::abort(send, recv);
        }
        self.send.take();
        self.recv.take();
        let _ = self
            .node
            .invalidate_outgoing_session(&self.session_key, self.generation);
        self.state = WriterState::Failed(error.kind(), error.to_string());
        error
    }
}

impl Write for PeerWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &self.state {
            WriterState::Committed => {
                return Err(eio("Peer-Schreibkanal ist bereits abgeschlossen"))
            }
            WriterState::Failed(kind, message) => {
                return Err(io::Error::new(*kind, message.clone()))
            }
            WriterState::Open => {}
        }
        for chunk in buf.chunks(fs::CHUNK) {
            let result = {
                let Some(send) = self.send.as_mut() else {
                    return Err(eio("Peer-Schreibkanal geschlossen"));
                };
                self.node.block_on(super::io_deadline::run(
                    "peer write data chunk",
                    send_tagged(send, TAG_DATA, chunk),
                ))
            };
            if let Err(error) = result {
                return Err(self.fail_open_stream(error));
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.finish()
    }
}

impl Drop for PeerWriter {
    fn drop(&mut self) {
        if matches!(self.state, WriterState::Open) {
            // Stream drop/reset makes the host abandon the write without
            // promotion. Only an explicit flush sends WriteDone; safe cleanup
            // additionally requires stable ownership proof.
            drop(self.send.take());
            drop(self.recv.take());
        }
    }
}
