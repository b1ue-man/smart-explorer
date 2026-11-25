//! The viewer's side of the picture streams (decision C3, NR N1): accepts
//! the host's uni streams, reads and checks their records, acknowledges each
//! frame right after its last record and hands the records to the decoder
//! in order. A continuation stream (`FLAG_CONTINUE`, plan 2026-11-14
//! „Bildkette“ B-36) is handed on only after every older stream was: the
//! frames the viewer acknowledged reach the decoder before the frame that
//! references them. A stream that starts with `SCRP` is the host's
//! bandwidth probe (`bandwidth_probe.rs`): it is counted and answered, and
//! no continuation waits for it. Split from `gop_stream.rs`.
use super::*;
use std::time::Duration;

use iroh::endpoint::RecvStream;
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, watch};

use super::super::pacing::STALE_STREAM_US;
use super::super::wire_binary::{self, read_gop_header, RecordSequence, StreamKind};
use super::super::wire_control::{FrameAck, ViewerFrame};
use super::super::{frame_budget, frame_memory, frame_size_caps};

/// Picture events a reader may queue ahead of the decoder; a slower
/// decoder makes the reader wait, so QUIC flow control throttles the host.
const PICTURE_QUEUE: usize = 4;

/// What the viewer's readers deliver.
pub enum PictureEvent {
    /// One record of a picture group; `received_us` right after it arrived.
    Record {
        header: GopHeader,
        record: FrameRecord,
        received_us: u64,
    },
    /// The stream of `gop_id` broke off; its partial frame is lost.
    Broken { gop_id: u64 },
}

/// The viewer's channel of picture events.
pub fn picture_channel() -> (mpsc::Sender<PictureEvent>, mpsc::Receiver<PictureEvent>) {
    mpsc::channel(PICTURE_QUEUE)
}

/// Accepts the host's picture streams until the connection ends; each runs
/// in a task of its own. `acks` takes the acknowledgements for the control
/// stream's writer.
pub async fn accept_picture_streams(
    connection: Connection,
    events: mpsc::Sender<PictureEvent>,
    acks: mpsc::Sender<ViewerFrame>,
) {
    accept_picture_streams_with_contract(connection, events, acks, None).await;
}

pub async fn accept_picture_streams_with_contract(
    connection: Connection,
    events: mpsc::Sender<PictureEvent>,
    acks: mpsc::Sender<ViewerFrame>,
    contract: Option<frame_budget::FrameContract>,
) {
    accept_picture_streams_with_memory(connection, events, acks, contract, None).await;
}
pub async fn accept_picture_streams_with_memory(
    connection: Connection,
    events: mpsc::Sender<PictureEvent>,
    acks: mpsc::Sender<ViewerFrame>,
    contract: Option<frame_budget::FrameContract>,
    memory: frame_memory::MemoryHold,
) {
    let local = match contract {
        Some(contract) => Some(contract.limits),
        None => match memory
            .as_ref()
            .map(|lease| frame_budget::legacy_limits(lease.bytes(), None))
            .transpose()
        {
            Ok(limits) => limits,
            Err(_) => return,
        },
    };
    accept_picture_streams_admitted(connection, events, acks, contract, memory, local).await;
}
pub async fn accept_picture_streams_admitted(
    connection: Connection,
    events: mpsc::Sender<PictureEvent>,
    acks: mpsc::Sender<ViewerFrame>,
    contract: Option<frame_budget::FrameContract>,
    memory: frame_memory::MemoryHold,
    local: Option<frame_budget::FrameLimits>,
) {
    // The probe stream has a slot of its own (`bandwidth_probe::PROBE_STREAMS`).
    let slots = VIEWER_UNI_STREAMS + super::super::bandwidth_probe::PROBE_STREAMS;
    let readers = std::sync::Arc::new(tokio::sync::Semaphore::new(slots as usize));
    // Active readers whose records a continuation stream must wait for.
    let mut active: Vec<watch::Receiver<bool>> = Vec::new();
    while let Ok(recv) = connection.accept_uni().await {
        let Ok(permit) = readers.clone().acquire_owned().await else {
            return;
        };
        active.retain(|rx| !*rx.borrow() && rx.has_changed().is_ok());
        let (events, acks) = (events.clone(), acks.clone());
        let memory = memory.clone();
        let (done, finished) = watch::channel(false);
        let before = active.clone();
        active.push(finished);
        let line = connection.clone();
        tokio::spawn(async move {
            let mut recv = recv;
            let mut magic = [0u8; 4];
            if recv.read_exact(&mut magic).await.is_err() {
                let _ = recv.stop(STREAM_RESET_CODE);
                let _ = done.send(true);
                return;
            }
            if magic == super::super::bandwidth_probe::PROBE_MAGIC {
                // No continuation waits for the probe.
                let _ = done.send(true);
                super::super::bandwidth_probe::read_probe(recv, &line, &acks).await;
                drop((permit, memory));
                return;
            }
            let bytes = memory.as_ref().map(|lease| lease.bytes());
            let streams = (recv, before);
            let broken = events.clone();
            let gop =
                read_picture_stream(streams, magic, events, acks, contract, bytes, local).await;
            // The slot of a broken stream comes free before its `Broken`
            // waits for the decoder (Gemini 3/3: four broken readers that
            // waited held every slot, and the host's next stream timed out).
            drop((permit, memory));
            if let Some(gop_id) = gop {
                let _ = broken.send(PictureEvent::Broken { gop_id }).await;
            }
            let _ = done.send(true);
        });
    }
}

/// Waits until every older stream handed its records on; `false` when one
/// did not within `STALE_STREAM_US` (the host resets a stream it gives up,
/// so its reader ends within a few round trips).
async fn older_ended(before: Vec<watch::Receiver<bool>>) -> bool {
    // A reader that is gone (its task ended) counts as ended.
    tokio::time::timeout(Duration::from_micros(STALE_STREAM_US), async move {
        for mut rx in before {
            let _ = rx.wait_for(|ended| *ended).await;
        }
    })
    .await
    .is_ok()
}

async fn read_picture_stream(
    (mut recv, mut before): (RecvStream, Vec<watch::Receiver<bool>>),
    magic: [u8; 4],
    events: mpsc::Sender<PictureEvent>,
    acks: mpsc::Sender<ViewerFrame>,
    contract: Option<frame_budget::FrameContract>,
    memory_bytes: Option<u64>,
    local: Option<frame_budget::FrameLimits>,
) -> Option<u64> {
    // The magic was read to tell a picture stream from the probe.
    let header = read_gop_header(&mut (&magic[..]).chain(&mut recv)).await;
    let header = match header {
        Ok(header) => header,
        Err(_) => {
            let _ = recv.stop(STREAM_RESET_CODE);
            return None;
        }
    };
    let contract_matches = frame_budget::contract_matches(header.frame_contract, contract);
    let exceeds = memory_bytes.is_some_and(|bytes| {
        let owners = if header.frame_contract.is_some() {
            frame_budget::VERSIONED_WORKING_IMAGES
        } else {
            frame_budget::WORKING_IMAGES
        };
        header.size.pixels().saturating_mul(4 * owners) > bytes
    });
    if !contract_matches
        || exceeds
        || local.is_some_and(|limits| limits.check(header.size).is_err())
    {
        let _ = recv.stop(STREAM_RESET_CODE);
        return None;
    }
    let Ok(budget) = frame_size_caps::PacketBudget::for_receive(header.frame_contract, local)
    else {
        let _ = recv.stop(STREAM_RESET_CODE);
        return None;
    };
    let mut sequence = RecordSequence::default();
    let mut allocation_budget = wire_binary::FrameReadBudget::default();
    let mut current = 0;
    let mut records = 0usize;
    let mut bytes = 0usize;
    loop {
        let record = match wire_binary::read_frame_record_admitted(
            &mut recv,
            &header,
            &mut allocation_budget,
            budget,
        )
        .await
        {
            Ok(Some(record)) => record,
            Ok(None) if !sequence.is_open() => return None,
            Ok(None) | Err(_) => break,
        };
        if current != record.frame_id {
            current = record.frame_id;
            records = 0;
            bytes = 0;
        }
        records = records.saturating_add(1);
        bytes = bytes
            .saturating_add(record.payload.len())
            .saturating_add(RECORD_HEADER_BYTES);
        if records > budget.records || bytes > budget.encoded_bytes {
            break;
        }
        let received_us = now_us();
        if sequence.accept(&header, &record).is_err() {
            break;
        }
        // B-36: the acknowledged frames a continuation references reach the
        // decoder first; without that order the stream counts as broken.
        if record.is_continuation() && !before.is_empty() {
            if !older_ended(std::mem::take(&mut before)).await {
                break;
            }
        }
        if !record.has_more() {
            let ack = FrameAck {
                gop_id: header.gop_id,
                frame_id: record.frame_id,
                received_us,
            };
            let frame = match header.kind {
                StreamKind::Video => Some(ViewerFrame::FrameAck(ack)),
                StreamKind::Preview => Some(ViewerFrame::PreviewAck(ack)),
                StreamKind::Snapshot => None,
            };
            if let Some(frame) = frame {
                if acks.send(frame).await.is_err() {
                    break;
                }
            }
        }
        let event = PictureEvent::Record {
            header,
            record,
            received_us,
        };
        if events.send(event).await.is_err() {
            break;
        }
    }
    // A broken stream is stopped at once, so its slot comes free (NR N1).
    let _ = recv.stop(STREAM_RESET_CODE);
    Some(header.gop_id)
}
