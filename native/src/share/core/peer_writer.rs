use std::io::{self, Write};
use std::sync::Arc;

use iroh::endpoint::{RecvStream, SendStream};

use super::core::eio;
use super::framing::{recv_resp, send_ctrl, send_tagged, TAG_DATA};
use super::fs;
use super::node::ShareIrohNode;
use super::wire::{Ctrl, FsRequest, FsResponse};

pub(super) fn writer(
    node: Arc<ShareIrohNode>,
    send: SendStream,
    recv: RecvStream,
) -> Box<dyn Write + Send> {
    Box::new(PeerWriter {
        node,
        send: Some(send),
        recv: Some(recv),
        finished: false,
    })
}

struct PeerWriter {
    node: Arc<ShareIrohNode>,
    send: Option<SendStream>,
    recv: Option<RecvStream>,
    finished: bool,
}

impl PeerWriter {
    fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        let Some(mut send) = self.send.take() else {
            return Err(eio("Peer-Schreibkanal geschlossen"));
        };
        let Some(mut recv) = self.recv.take() else {
            return Err(eio("Peer-Schreibantwort geschlossen"));
        };
        self.node.block_on(async {
            send_ctrl(
                &mut send,
                &Ctrl::Fs {
                    req: FsRequest::WriteDone,
                },
            )
            .await?;
            match recv_resp(&mut recv).await? {
                FsResponse::Ok => {
                    send.finish().map_err(eio)?;
                    Ok(())
                }
                _ => Err(eio("unerwartete Antwort auf Schreib-Ende")),
            }
        })?;
        self.finished = true;
        Ok(())
    }
}

impl Write for PeerWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.finished {
            return Err(eio("Peer-Schreibkanal ist bereits abgeschlossen"));
        }
        let Some(send) = self.send.as_mut() else {
            return Err(eio("Peer-Schreibkanal geschlossen"));
        };
        self.node.block_on(async {
            for chunk in buf.chunks(fs::CHUNK) {
                send_tagged(send, TAG_DATA, chunk).await?;
            }
            Ok::<(), io::Error>(())
        })?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.finish()
    }
}

impl Drop for PeerWriter {
    fn drop(&mut self) {
        if !self.finished {
            self.finished = true;
            // Stream drop/reset makes the host abandon and remove its staged
            // file. Only an explicit flush sends WriteDone and promotes it.
            drop(self.send.take());
            drop(self.recv.take());
        }
    }
}
