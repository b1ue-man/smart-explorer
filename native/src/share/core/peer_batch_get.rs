//! Client side of GetBatch: one stream delivers many small files to a
//! `BatchSink` in request order. An item may fail on its own; a transport
//! failure ends the call, and items without `end`/`failed` were not
//! delivered (reads are safe to repeat).
use std::io;
use std::ops::Range;

use iroh::endpoint::RecvStream;

use crate::share::core::eio;
use crate::share::framing::{recv_data_frame, recv_resp_wire, send_ctrl, TAG_CTRL, TAG_DATA};
use crate::share::fs_error::into_io;
use crate::share::io_deadline;
use crate::share::node_sessions::OpenedPeerStream;
use crate::share::wire::{BatchPart, Ctrl, FsBatchGet, FsRequest, FsResponse};
use crate::vfs::{BatchGet, BatchSink, VfsResult};

use super::peer_transfer::plan_request;
use super::PeerBackend;

enum GetFailure {
    /// The sink refused; the stream is dropped, the connection stays.
    Local(io::Error),
    /// The stream or the protocol failed; the connection is replaced.
    Transport(io::Error),
    /// The host refused the whole batch (for example `Busy`).
    Refused(io::Error),
}

impl PeerBackend {
    pub(super) fn get_batch_v1(
        &self,
        items: &[BatchGet],
        sink: &mut dyn BatchSink,
    ) -> VfsResult<()> {
        let limits = self.batch_limits_required()?;
        let lease = self.mount_lease_token()?;
        let wire: Vec<FsBatchGet> = items
            .iter()
            .map(|item| FsBatchGet {
                path: item.path.clone(),
                id: item.id.clone(),
                size: item.size,
            })
            .collect();
        let envelope = get_header(Vec::new(), lease.clone());
        for part in plan_request(&envelope, &wire, |item| item.size, limits)? {
            match part {
                BatchPart::Oversized(index) => {
                    let error = io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Datei passt in kein Paket; sie wird einzeln übertragen",
                    );
                    sink.failed(index, error)?;
                }
                BatchPart::Items(range) => self.get_part(&wire, range, &lease, sink)?,
            }
        }
        Ok(())
    }

    fn get_part(
        &self,
        items: &[FsBatchGet],
        range: Range<usize>,
        lease: &Option<String>,
        sink: &mut dyn BatchSink,
    ) -> io::Result<()> {
        let Some(part) = items.get(range.clone()) else {
            return Err(eio("Paketbereich ist ungültig"));
        };
        let mut opened = self.open_batch_stream()?;
        let header = get_header(part.to_vec(), lease.clone());
        match self.receive_part(&mut opened, &header, range, sink) {
            Ok(()) => Ok(()),
            Err(GetFailure::Refused(error)) => Err(error),
            Err(GetFailure::Local(error)) => {
                io_deadline::abort(&mut opened.send, &mut opened.recv);
                Err(error)
            }
            Err(GetFailure::Transport(error)) => {
                io_deadline::abort(&mut opened.send, &mut opened.recv);
                let _ = self
                    .node
                    .invalidate_outgoing_session(&opened.session_key, opened.generation);
                Err(error)
            }
        }
    }

    fn receive_part(
        &self,
        opened: &mut OpenedPeerStream,
        header: &Ctrl,
        range: Range<usize>,
        sink: &mut dyn BatchSink,
    ) -> Result<(), GetFailure> {
        self.node
            .block_on(io_deadline::run(
                "peer batch request",
                send_ctrl(&mut opened.send, header),
            ))
            .map_err(GetFailure::Transport)?;
        match self.next_response(&mut opened.recv)? {
            FsResponse::Ready => {}
            FsResponse::Err { kind, msg } => return Err(GetFailure::Refused(into_io(kind, msg))),
            _ => {
                return Err(GetFailure::Transport(eio(
                    "unerwartete Antwort auf das Paket",
                )))
            }
        }
        for index in range {
            self.receive_item(&mut opened.recv, index, sink)?;
        }
        Ok(())
    }

    fn next_response(&self, recv: &mut RecvStream) -> Result<FsResponse, GetFailure> {
        self.node
            .block_on(io_deadline::run("peer batch reply", recv_resp_wire(recv)))
            .map_err(GetFailure::Transport)
    }

    /// Header (length or failure), exactly that many bytes, then the result.
    fn receive_item(
        &self,
        recv: &mut RecvStream,
        index: usize,
        sink: &mut dyn BatchSink,
    ) -> Result<(), GetFailure> {
        let size = match self.next_response(recv)? {
            FsResponse::Data { size } => size,
            FsResponse::Err { kind, msg } => {
                return sink
                    .failed(index, into_io(kind, msg))
                    .map_err(GetFailure::Local)
            }
            _ => return Err(GetFailure::Transport(eio("unerwartete Antwort im Paket"))),
        };
        sink.begin(index, size).map_err(GetFailure::Local)?;
        let mut remaining = size;
        while remaining > 0 {
            let (tag, payload) = self
                .node
                .block_on(io_deadline::run("peer batch data", recv_data_frame(recv)))
                .map_err(GetFailure::Transport)?;
            if tag != TAG_DATA {
                // The host ends an item early only with its failure.
                let error = early_failure(tag, &payload)?;
                return sink.end(index, Err(error)).map_err(GetFailure::Local);
            }
            let length = payload.len() as u64;
            if length > remaining {
                return Err(GetFailure::Transport(eio(
                    "Peer sendet mehr Paketdaten als angekündigt",
                )));
            }
            remaining -= length;
            sink.data(index, &payload).map_err(GetFailure::Local)?;
        }
        let result = match self.next_response(recv)? {
            FsResponse::Ok => Ok(()),
            FsResponse::Err { kind, msg } => Err(into_io(kind, msg)),
            _ => {
                return Err(GetFailure::Transport(eio(
                    "unerwartetes Ende eines Paketeintrags",
                )))
            }
        };
        sink.end(index, result).map_err(GetFailure::Local)
    }
}

fn early_failure(tag: u8, payload: &[u8]) -> Result<io::Error, GetFailure> {
    if tag != TAG_CTRL {
        return Err(GetFailure::Transport(eio("unerwarteter Frame im Paket")));
    }
    match serde_json::from_slice::<Ctrl>(payload) {
        Ok(Ctrl::FsResp {
            resp: FsResponse::Err { kind, msg },
        }) => Ok(into_io(kind, msg)),
        Ok(_) => Err(GetFailure::Transport(eio("Paketeintrag endet zu früh"))),
        Err(error) => Err(GetFailure::Transport(eio(error))),
    }
}

fn get_header(items: Vec<FsBatchGet>, lease: Option<String>) -> Ctrl {
    Ctrl::Fs {
        req: FsRequest::GetBatch { items },
        lease,
    }
}
