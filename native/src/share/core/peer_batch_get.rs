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
use crate::share::wire::{BatchPart, Ctrl, FsBatchGet, FsFileEvidence, FsRequest, FsResponse};
use crate::vfs::{BatchGet, BatchSink, VfsResult};

use super::peer_evidence::EvidenceBinding;
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
        // `evidence_v1`: each item header carries what the host read, bound
        // to this connection like a FileIdentity reply.
        let binding = if self.evidence_offered() {
            Some(EvidenceBinding::capture(self)?)
        } else {
            None
        };
        let envelope = get_header(Vec::new(), lease.clone(), binding.is_some());
        for part in plan_request(&envelope, &wire, |item| item.size, limits)? {
            match part {
                BatchPart::Oversized(index) => {
                    let error = io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Datei passt in kein Paket; sie wird einzeln übertragen",
                    );
                    sink.failed(index, error)?;
                }
                BatchPart::Items(range) => {
                    self.get_part(&wire, range, &lease, binding.as_ref(), sink)?
                }
            }
        }
        Ok(())
    }

    fn get_part(
        &self,
        items: &[FsBatchGet],
        range: Range<usize>,
        lease: &Option<String>,
        binding: Option<&EvidenceBinding>,
        sink: &mut dyn BatchSink,
    ) -> io::Result<()> {
        let Some(part) = items.get(range.clone()) else {
            return Err(eio("Paketbereich ist ungültig"));
        };
        let mut opened = self.open_batch_stream()?;
        let header = get_header(part.to_vec(), lease.clone(), binding.is_some());
        let mut receiver = Receiver {
            items,
            binding,
            sink,
        };
        match self.receive_part(&mut opened, &header, range, &mut receiver) {
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
        receiver: &mut Receiver<'_, '_>,
    ) -> Result<(), GetFailure> {
        self.node
            .block_on_route(
                opened.generation,
                io_deadline::run("peer batch request", send_ctrl(&mut opened.send, header)),
            )
            .map_err(GetFailure::Transport)?;
        match self.next_response(&mut opened.recv, opened.generation)? {
            FsResponse::Ready => {}
            FsResponse::Err { kind, msg } => return Err(GetFailure::Refused(into_io(kind, msg))),
            _ => {
                return Err(GetFailure::Transport(eio(
                    "unerwartete Antwort auf das Paket",
                )))
            }
        }
        for index in range {
            self.receive_item(&mut opened.recv, opened.generation, index, receiver)?;
        }
        Ok(())
    }

    fn next_response(
        &self,
        recv: &mut RecvStream,
        generation: usize,
    ) -> Result<FsResponse, GetFailure> {
        self.node
            .block_on_route(
                generation,
                io_deadline::run("peer batch reply", recv_resp_wire(recv)),
            )
            .map_err(GetFailure::Transport)
    }

    /// Header (length or failure), exactly that many bytes, then the result.
    fn receive_item(
        &self,
        recv: &mut RecvStream,
        generation: usize,
        index: usize,
        receiver: &mut Receiver<'_, '_>,
    ) -> Result<(), GetFailure> {
        let size = match self.next_response(recv, generation)? {
            FsResponse::Data { size } => size,
            FsResponse::EvidencedData { size, evidence } => {
                receiver.evidence(self, index, evidence)?;
                size
            }
            FsResponse::Err { kind, msg } => {
                return receiver
                    .sink
                    .failed(index, into_io(kind, msg))
                    .map_err(GetFailure::Local)
            }
            _ => return Err(GetFailure::Transport(eio("unerwartete Antwort im Paket"))),
        };
        receiver
            .sink
            .begin(index, size)
            .map_err(GetFailure::Local)?;
        let mut remaining = size;
        while remaining > 0 {
            let (tag, payload) = self
                .node
                .block_on_route(
                    generation,
                    io_deadline::run("peer batch data", recv_data_frame(recv)),
                )
                .map_err(GetFailure::Transport)?;
            if tag != TAG_DATA {
                // The host ends an item early only with its failure.
                let error = early_failure(tag, &payload)?;
                return receiver
                    .sink
                    .end(index, Err(error))
                    .map_err(GetFailure::Local);
            }
            let length = payload.len() as u64;
            if length > remaining {
                return Err(GetFailure::Transport(eio(
                    "Peer sendet mehr Paketdaten als angekündigt",
                )));
            }
            remaining -= length;
            receiver
                .sink
                .data(index, &payload)
                .map_err(GetFailure::Local)?;
        }
        let result = match self.next_response(recv, generation)? {
            FsResponse::Ok => Ok(()),
            FsResponse::Err { kind, msg } => Err(into_io(kind, msg)),
            _ => {
                return Err(GetFailure::Transport(eio(
                    "unerwartetes Ende eines Paketeintrags",
                )))
            }
        };
        receiver.sink.end(index, result).map_err(GetFailure::Local)
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

fn get_header(items: Vec<FsBatchGet>, lease: Option<String>, evidence: bool) -> Ctrl {
    Ctrl::Fs {
        req: FsRequest::GetBatch { items, evidence },
        lease,
    }
}

/// The sink of one part and how its evidence is bound.
struct Receiver<'s, 'b> {
    items: &'s [FsBatchGet],
    binding: Option<&'b EvidenceBinding>,
    sink: &'s mut dyn BatchSink,
}

impl Receiver<'_, '_> {
    /// Hands the sink the item's evidence while the connection's binding
    /// still holds; otherwise the item goes on without it.
    fn evidence(
        &mut self,
        backend: &PeerBackend,
        index: usize,
        evidence: FsFileEvidence,
    ) -> Result<(), GetFailure> {
        let (Some(binding), Some(item)) = (self.binding, self.items.get(index)) else {
            return Ok(());
        };
        if !binding.still_current(backend) {
            return Ok(());
        }
        match binding.file(&item.path, evidence) {
            Some(evidence) => self
                .sink
                .evidence(index, evidence)
                .map_err(GetFailure::Local),
            None => Ok(()),
        }
    }
}
