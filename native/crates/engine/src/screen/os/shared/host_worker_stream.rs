//! The daemon side of the picture stream (plan 2026-11-14 „Bildkette“
//! B-27). With a host process that announced `STREAM_VERSION` a frame is no
//! request any more: `produce` grants one credit (and, when the pipeline's
//! queue has room, one picture ahead) in a `STREAM` order that also carries
//! the settings, the cadence and a key frame wish, and takes the next answer
//! of the process from the stream channel. The process captures at the
//! cadence on its own and may already have encoded this picture while the
//! previous one was sent. Neither the order nor the wait holds the link, so
//! input and control requests go on meanwhile. The deadline of a picture
//! stays `FRAME_TIMEOUT`; a process that ends, breaks the order of its
//! answers or misses the deadline ends the session as before.
use super::reader::StreamAnswer;
use super::*;
use crate::screen::core::chain_meter::{ChainMeter, Station, STATION_WINDOW};
use crate::screen::core::frame_timing::CaptureFacts;
use crate::screen::core::host_stage::{StreamHint, StreamReport};
use crate::screen::core::worker_wire::{StreamMark, StreamOrder, STREAM_VERSION};

/// The pipe ends of the picture stream.
pub(super) struct StreamPort {
    pub(super) input: Arc<Mutex<ChildStdin>>,
    pub(super) answers: Mutex<mpsc::Receiver<StreamAnswer>>,
}

/// Credit and key frame wishes of one stream, and what its answers report.
#[derive(Default)]
pub(super) struct StreamClient {
    granted: u64,
    key_serial: u64,
    key_pending: bool,
    dropped_seen: u64,
    hint: Option<StreamHint>,
    report: Option<StreamReport>,
    /// B-36: wishes to continue from a confirmed frame, and the last one.
    recover_serial: u64,
    last_good: u64,
    recover: Option<u64>,
    /// Station samples of the answers since the meter took them, and the
    /// latest capture facts with the read-back count (BK0).
    chain: Vec<(Station, u64)>,
    facts: Option<(CaptureFacts, u64)>,
}

impl StreamClient {
    /// What `mark` reports, the daemon having read it at `arrived_us`.
    fn note_chain(&mut self, mark: &StreamMark, arrived_us: u64) {
        if self.chain.len() + 4 > 4 * STATION_WINDOW {
            self.chain.drain(..4);
        }
        if mark.frame_id != 0 {
            // `encode_us` holds the conversion; the station `encode` not (r1).
            let convert = (mark.convert_us != 0).then_some(mark.convert_us);
            let encode =
                crate::screen::core::convert_clock::encode_without(mark.encode_us, convert);
            self.chain.push((Station::Capture, mark.capture_us));
            self.chain.push((Station::Encode, encode));
            if let Some(convert) = convert {
                self.chain.push((Station::Convert, convert));
            }
        }
        if mark.handed_us != 0 {
            let handoff = arrived_us.saturating_sub(mark.handed_us);
            self.chain.push((Station::WorkerHandoff, handoff));
        }
        let mut unpacked = CaptureFacts::unpack(mark.facts);
        if unpacked.on_gpu.is_none() {
            unpacked.on_gpu = self.facts.and_then(|(prev, _)| prev.on_gpu);
        }
        self.facts = Some((unpacked, mark.readbacks));
    }
}

impl SourceFacts {
    fn client(&self) -> MutexGuard<'_, StreamClient> {
        self.stream.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl WorkerSource {
    /// The process paces its own pictures.
    pub(super) fn streams(&self) -> bool {
        self.facts.stream_version.load(Ordering::Acquire) >= STREAM_VERSION
    }

    pub(super) fn keep_stream_hint(&self, hint: StreamHint) {
        self.facts.client().hint = Some(hint);
    }

    pub(super) fn last_stream_report(&self) -> Option<StreamReport> {
        self.facts.client().report.take()
    }

    /// Hands the station samples and facts of the answers to `meter` (BK0).
    pub(super) fn drain_stream_chain(&self, meter: &mut ChainMeter) {
        let mut client = self.facts.client();
        for (station, micros) in client.chain.drain(..) {
            meter.record_us(station, micros);
        }
        if let Some((facts, readbacks)) = client.facts.take() {
            meter.set_facts(facts);
            meter.set_readbacks(readbacks);
        }
    }

    /// B-36: the next order asks the process to continue from `last_good`.
    pub(super) fn keep_recovery(&self, last_good: u64) {
        self.facts.client().recover = Some(last_good);
    }

    /// One frame of the picture stream: one credit, one answer.
    pub(super) fn produce_streamed(
        &mut self,
        request: &StageRequest,
    ) -> Result<StageFrame, StageError> {
        let port = self
            .facts
            .stream_port
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or_else(|| {
                StageError::new(
                    ScreenErrorCode::CaptureFailed,
                    io::Error::other("the picture stream has no pipe"),
                )
            })?;
        let order = {
            let mut client = self.facts.client();
            client.granted += 1;
            if request.keyframe && !client.key_pending {
                client.key_serial += 1;
                client.key_pending = true;
            }
            let hint = client.hint.take().unwrap_or_default();
            if let Some(last_good) = client.recover.take() {
                client.recover_serial += 1;
                client.last_good = last_good;
            }
            StreamOrder {
                request: request.clone(),
                granted: client.granted,
                ahead: u32::from(hint.ahead),
                key_serial: client.key_serial,
                cadence_fps: if hint.cadence_fps == 0 {
                    request.fps
                } else {
                    hint.cadence_fps
                },
                recover_serial: client.recover_serial,
                last_good: client.last_good,
                quality_target: hint.quality_target,
            }
        };
        let granted = order.granted;
        self.link().check_session()?;
        let sent = port
            .input
            .lock()
            .map_err(|_| io::Error::other("worker input poisoned"))
            .and_then(|mut input| send_request(&mut *input, &WorkerRequest::Stream(order)));
        if let Err(error) = sent {
            return Err(self.link().lost(error));
        }
        let answers = port.answers.lock().unwrap_or_else(PoisonError::into_inner);
        let (mark, reply) = match answers.recv_timeout(FRAME_TIMEOUT) {
            Ok(Ok(Some(answer))) => answer,
            Ok(Ok(None)) | Err(RecvTimeoutError::Disconnected) => {
                return Err(self
                    .link()
                    .lost(io::Error::from(io::ErrorKind::UnexpectedEof)))
            }
            Ok(Err(error)) => return Err(self.link().lost(error)),
            Err(RecvTimeoutError::Timeout) => {
                self.link().kill();
                let text = format!(
                    "the screen host process did not answer within {} s",
                    FRAME_TIMEOUT.as_secs()
                );
                return Err(StageError::new(
                    ScreenErrorCode::CaptureFailed,
                    io::Error::new(io::ErrorKind::TimedOut, text),
                ));
            }
        };
        drop(answers);
        let arrived_us = crate::screen::core::gop_stream::now_us();
        if mark.answer != granted {
            return Err(self.unexpected("STREAM (answer out of order)"));
        }
        let reply = self.link().accept(reply)?;
        {
            let mut client = self.facts.client();
            let dropped = mark.dropped.saturating_sub(client.dropped_seen);
            client.dropped_seen = client.dropped_seen.max(mark.dropped);
            client.report = Some(StreamReport {
                frame_id: (mark.frame_id != 0).then_some(mark.frame_id),
                capture_us: mark.capture_us,
                encode_us: mark.encode_us,
                dropped_before_encode: dropped,
            });
            client.note_chain(&mark, arrived_us);
            if matches!(
                reply,
                WorkerReply::Frame(StageFrame::Encoded { keyframe: true, .. })
            ) {
                client.key_pending = false;
            }
        }
        match reply {
            WorkerReply::Frame(frame) => {
                match &frame {
                    StageFrame::Encoded { source_size, .. }
                    | StageFrame::Unchanged { source_size } => self.size = *source_size,
                    StageFrame::Paused(_) => {}
                }
                Ok(frame)
            }
            WorkerReply::Failed { code, message } => {
                Err(StageError::new(code, io::Error::other(message)))
            }
            _ => Err(self.unexpected("STREAM")),
        }
    }
}

#[cfg(test)]
#[path = "bildkette_chain_crash_task_tests.rs"]
mod crash_tests;
