//! The encoding stage of the picture stream (plan 2026-11-14 „Bildkette“
//! B-27): scales and encodes the waiting picture for credit, falls back to
//! the next encoder of the same codec once (`refused`, as `PRODUCE` does),
//! prepares the owned transport and answers each credit exactly once with a
//! `STREAMED` mark ahead of the answer. With `ahead` it encodes one more
//! picture before its credit and holds it, transport included, until that
//! credit comes; a key frame wish or another encoder drops it instead. Only
//! this stage writes stream answers, so every answer follows its mark. A
//! recovery wish (B-36, `StreamOrder::recover_serial`) drops a picture
//! encoded before it like a key frame wish; the next picture continues from
//! the confirmed frame, or is a key frame of a new encoder when the encoder
//! cannot (`ScreenEncoder::recover_references`).
use super::*;
use crate::screen::core::capture_admission::CaptureOwner;
use crate::screen::core::capture_bytes::AdmittedBytes;
use crate::screen::core::capture_transport;
use crate::screen::core::codec_registry::OpenedEncoder;
use crate::screen::core::ref_recovery::Recovery;
use crate::screen::core::worker_wire::{send_mark, StreamMark};

/// Opens the encoder for a stream; the codecs of this user, or a test's.
pub(in crate::screen::os) trait EncoderOpener: Send + Sync {
    fn open(
        &self,
        request: &StageRequest,
        refused: &[String],
        grant: CaptureGrant,
    ) -> io::Result<OpenedEncoder>;

    /// Number of rank stages available for `request` (W4, B3).
    fn stage_count(&self, _request: &StageRequest) -> usize {
        1
    }
}

/// The codecs this user has (`codec_inventory`).
pub(in super::super) struct Registry;

impl EncoderOpener for Registry {
    fn open(
        &self,
        request: &StageRequest,
        refused: &[String],
        grant: CaptureGrant,
    ) -> io::Result<OpenedEncoder> {
        codec_inventory::registry().open_encoder_for_stage_admitted(request, refused, grant)
    }

    fn stage_count(&self, request: &StageRequest) -> usize {
        codec_inventory::registry().encoder_stage_count(request)
    }
}

/// A picture encoded and serialized ahead of its credit.
pub(super) struct Held {
    /// The picture outlives the write of its serialization, as in `PRODUCE`.
    _reply: WorkerReply,
    bytes: AdmittedBytes,
    pub(super) keyframe: bool,
    frame_id: u64,
    capture_us: u64,
    encode_us: u64,
    /// The conversion within `encode_us` as its own call (r1); 0 without.
    convert_us: u64,
    /// The picture reached the encoder on the GPU (BK0, `CaptureFacts`).
    on_gpu: bool,
}

enum Answer {
    Picture(Held),
    Plain(WorkerReply),
}

enum Work {
    Deliver(StreamMark, Answer),
    Encode {
        captured: Captured,
        request: StageRequest,
        /// The open recovery wish (B-36) the encoding answers.
        recover: Option<(u64, u64)>,
    },
}

fn mark(state: &State, answer: &Answer) -> StreamMark {
    // What the capture of this process reports of itself (BK0).
    let mut facts = crate::screen::os::capture_facts();
    let readbacks = crate::screen::core::capture_picture::readbacks();
    match answer {
        Answer::Picture(held) => {
            facts.on_gpu = Some(held.on_gpu);
            StreamMark {
                answer: state.answered,
                frame_id: held.frame_id,
                capture_us: held.capture_us,
                encode_us: held.encode_us,
                convert_us: held.convert_us,
                dropped: state.dropped,
                facts: facts.pack() | crate::screen::os::capture_fallback_bits(),
                readbacks,
                ..StreamMark::default()
            }
        }
        Answer::Plain(_) => StreamMark {
            answer: state.answered,
            dropped: state.dropped,
            facts: facts.pack() | crate::screen::os::capture_fallback_bits(),
            readbacks,
            ..StreamMark::default()
        },
    }
}

impl State {
    /// The request an encoding uses: the latest settings, a key frame while
    /// one is wanted.
    fn encode_request(&self) -> Option<StageRequest> {
        let mut request = self.order.as_ref()?.request.clone();
        request.keyframe = self.key_wanted;
        Some(request)
    }

    /// Takes the waiting picture for encoding; captures from now on carry
    /// the next frame id.
    fn take_slot(&mut self) -> Option<Captured> {
        let captured = self.slot.take()?;
        self.next_id = self.next_id.max(captured.frame_id.saturating_add(1));
        self.encoding = true;
        Some(captured)
    }

    /// What the encoding stage does next; it owns the work outside the lock.
    fn next_work(&mut self) -> Option<Work> {
        if self.quiet > 0 {
            return None;
        }
        if self.answered < self.granted {
            let answer = if let Some(held) = self.held.take() {
                Answer::Picture(held)
            } else if let Some((reply, _)) = self.event.take() {
                Answer::Plain(reply)
            } else if self.slot_fresh() {
                return self.encode_work();
            } else if self.slot.is_some() {
                // A stale picture waits for the capture that replaces it.
                return None;
            } else {
                Answer::Plain(self.unchanged.take()?)
            };
            self.answered += 1;
            self.encoding = true;
            return Some(Work::Deliver(mark(self, &answer), answer));
        }
        let ahead = self.order.as_ref().is_some_and(|order| order.ahead > 0);
        if ahead && self.held.is_none() && self.slot_fresh() && !self.failed {
            return self.encode_work();
        }
        None
    }

    /// The waiting picture with the latest settings and the open recovery.
    fn encode_work(&mut self) -> Option<Work> {
        let request = self.encode_request()?;
        let captured = self.take_slot()?;
        let recover = self.recover;
        Some(Work::Encode {
            captured,
            request,
            recover,
        })
    }

    /// Takes the result of an encoding begun under `serial`: an answer to
    /// deliver now, or a picture held for its credit, or what goes.
    /// `asked_recover` is the recovery wish (B-36) the encoding answered.
    fn settle(
        &mut self,
        result: Result<Held, WorkerReply>,
        serial: u64,
        asked_key: bool,
        asked_recover: Option<u64>,
    ) -> (Option<Work>, Option<Held>) {
        if self.serial != serial || self.stopping || self.quiet > 0 {
            // Its encoder moved on without the viewer: a key frame follows.
            if result.is_ok() {
                self.key_wanted = true;
            }
            return (None, result.ok());
        }
        let answer = match result {
            Ok(held) => {
                if held.keyframe {
                    self.key_wanted = false;
                    self.recover = None;
                } else if self.key_wanted && !asked_key {
                    // A key frame was wished while this one was encoded.
                    return (None, Some(held));
                } else if let Some((wish, _)) = self.recover {
                    // Encoded before the wish, it references lost frames.
                    if asked_recover != Some(wish) {
                        return (None, Some(held));
                    }
                    self.recover = None;
                }
                if self.answered >= self.granted {
                    self.held = Some(held);
                    return (None, None);
                }
                Answer::Picture(held)
            }
            Err(reply) => {
                if self.answered >= self.granted {
                    // Ahead of its credit only a failure stays for it.
                    if matches!(reply, WorkerReply::Reference { ref reply, .. }
                        if matches!(**reply, WorkerReply::Failed { .. }))
                    {
                        self.failed = true;
                        self.event = Some((reply, true));
                    }
                    return (None, None);
                }
                Answer::Plain(reply)
            }
        };
        self.answered += 1;
        (Some(Work::Deliver(mark(self, &answer), answer)), None)
    }
}

/// Answers the open credit with `reply` from outside the stages (they did
/// not start) and ends the stream.
pub(super) fn fail(shared: &Shared, reply: WorkerReply) {
    let pending = {
        let mut state = shared.lock();
        state.failed = true;
        if state.answered >= state.granted {
            state.event = Some((reply, true));
            return;
        }
        state.answered += 1;
        let answer = Answer::Plain(reply);
        (mark(&state, &answer), answer)
    };
    deliver(shared, pending.0, pending.1);
}

/// Writes `mark` and `answer` under one output lock; the answer is dropped
/// after the lock is gone (its release can write to the daemon).
fn deliver(shared: &Shared, mut mark: StreamMark, answer: Answer) {
    let written = (|| -> io::Result<()> {
        let mut output = shared
            .output
            .lock()
            .map_err(|_| io::Error::other("worker output poisoned"))?;
        // Station `worker_handoff` starts here (BK0).
        mark.handed_us = crate::screen::core::gop_stream::now_us();
        send_mark(&mut *output, &mark)?;
        match &answer {
            Answer::Picture(held) => capture_transport::send_prepared(&mut *output, &held.bytes),
            Answer::Plain(reply) => send_reply(&mut *output, reply),
        }
    })();
    drop(answer);
    if let Err(error) = written {
        // The daemon is gone; the end of the input ends this process.
        eprintln!("screen host process: {error}");
        shared.lock().failed = true;
    }
}

/// The encoding stage.
pub(super) fn encode_loop(shared: Arc<Shared>) {
    crate::screen::os::worker_thread::raise_priority();
    let mut encoding = Encoding::new(shared.opener.clone());
    loop {
        let work = {
            let mut state = shared.lock();
            loop {
                if state.stopping {
                    return;
                }
                if state.reset_encoder {
                    state.reset_encoder = false;
                    encoding.running = None;
                }
                state.needs_refresh = encoding.needs_refresh();
                if let Some(work) = state.next_work() {
                    break work;
                }
                state = shared.wait(state, RECHECK);
            }
        };
        let (deliver_now, gone) = match work {
            Work::Deliver(mark, answer) => (Some((mark, answer)), None),
            Work::Encode {
                captured,
                request,
                recover,
            } => {
                let serial = captured.serial;
                let last_good = recover.map(|(_, last_good)| last_good);
                let result = encoding.encode(&shared, &request, captured, last_good);
                let mut state = shared.lock();
                let wish = recover.map(|(wish, _)| wish);
                let (work, gone) = state.settle(result, serial, request.keyframe, wish);
                match work {
                    Some(Work::Deliver(mark, answer)) => (Some((mark, answer)), gone),
                    _ => (None, gone),
                }
            }
        };
        drop(gone);
        if let Some((mark, answer)) = deliver_now {
            deliver(&shared, mark, answer);
        }
        let mut state = shared.lock();
        state.encoding = false;
        shared.changed.notify_all();
    }
}

/// The encoder of the stream and what it was opened for.
pub(in crate::screen::os) struct Encoding {
    running: Option<Running>,
    pub(in crate::screen::os) refused: Vec<String>,
    checked: Instant,
    produced: u64,
    opener: Arc<dyn EncoderOpener>,
}

#[path = "host_worker_encoding.rs"]
mod encoding;

impl Encoding {
    /// Scales, encodes and serializes one picture; `Err` is the answer
    /// instead of a picture (unchanged or failed), wrapped in its reference.
    /// `recover` (B-36): the picture may reference only that confirmed frame.
    fn encode(
        &mut self,
        shared: &Shared,
        request: &StageRequest,
        captured: Captured,
        recover: Option<u64>,
    ) -> Result<Held, WorkerReply> {
        self.crash_hook();
        self.check_openh264();
        let started = Instant::now();
        let Captured {
            image,
            at: _,
            frame_id,
            capture_ts_us,
            capture_us,
            serial: _,
            grant,
            limits,
            generation,
            reference,
        } = captured;
        let source_size = image.size();
        let on_gpu = image.on_gpu();
        let unchanged =
            || reference.wrap(WorkerReply::Frame(StageFrame::Unchanged { source_size }));
        if request.size.width > source_size.width || request.size.height > source_size.height {
            return Err(unchanged());
        }
        let image = if image.size() == request.size {
            image
        } else {
            // A CPU picture is scaled on the CPU; a GPU picture only gets the
            // size the encoder's GPU scales it to (BK3b).
            let scaled = image.scaled(request.size, limits, CaptureOwner::ScaledPixels);
            // The captured pixels go before encoding: room for the next capture.
            drop(image);
            match scaled {
                Ok(scaled) => scaled,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Err(unchanged()),
                Err(error) => {
                    return Err(reference.wrap(failed(ScreenErrorCode::CaptureFailed, error)))
                }
            }
        };
        let Some(grant) = grant else {
            let error = "worker source has no capture grant";
            return Err(reference.wrap(failed(ScreenErrorCode::EncoderFailed, error)));
        };
        // B-36: the running encoder references the confirmed frame; one that
        // cannot gives way to a new encoder whose first picture is a key frame.
        let mut request = std::borrow::Cow::Borrowed(request);
        // Merge W2: the encoder stamps the picture with the id the viewer
        // acknowledges (`StreamMark::frame_id` = the captured one), not with
        // the order's; NVENC's B-36 bookkeeping (`RefHistory`) keys on it.
        if request.frame_id != frame_id {
            request.to_mut().frame_id = frame_id;
        }
        if let Some(last_good) = recover.filter(|_| !request.keyframe) {
            let continued = self.running.as_mut().is_some_and(|running| {
                running.encoder.recover_references(last_good) == Recovery::Continued
            });
            if !continued {
                self.running = None;
                request.to_mut().keyframe = true;
            }
        }
        // W4 (B3): every stage of the rank once; the stage arms the station
        // `convert` clock around its own conversion call (r1).
        let readbacks_before = crate::screen::core::capture_picture::readbacks();
        let encoded = self.encode_with_fallback(&image, request.clone(), &grant);
        let on_gpu =
            on_gpu && crate::screen::core::capture_picture::readbacks() == readbacks_before;
        let convert_us = crate::screen::core::convert_clock::take().unwrap_or(0);
        let (frame, name, hardware) = match encoded {
            Ok(encoded) => encoded,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Err(unchanged()),
            Err(error) => return Err(reference.wrap(failed(ScreenErrorCode::EncoderFailed, error))),
        };
        drop(image);
        if frame.records.is_empty() {
            return Err(unchanged());
        }
        if let Err(error) = crate::screen::core::frame_size_caps::validate_encoded_budget(
            request.frame_contract,
            request.choice.codec,
            &frame.records,
        ) {
            // The encoder references this picture, the viewer never gets it:
            // the next picture comes from a new encoder as a key frame.
            self.running = None;
            return Err(reference.wrap(failed(ScreenErrorCode::EncoderFailed, error)));
        }
        let keyframe = frame.keyframe;
        let picture = WorkerReply::Frame(StageFrame::Encoded {
            records: frame.records,
            keyframe,
            capture_ts_us,
            encoder: StageEncoder { name, hardware },
            source_size,
        });
        // The picture keeps the reference it was captured under.
        let reply = super::super::run::Reference {
            generation,
            ..reference
        }
        .wrap(picture);
        let prepared = (|| -> io::Result<Option<AdmittedBytes>> {
            let grant = capture_transport::picture_grant(&reply)?
                .ok_or_else(|| io::Error::other("worker picture has no grant"))?;
            let id = shared.remote.id_for(&grant)?;
            capture_transport::prepare_owned(&reply, id, &grant)
        })();
        let encode_us = micros(started.elapsed());
        match prepared {
            Ok(Some(bytes)) => Ok(Held {
                _reply: reply,
                bytes,
                keyframe,
                frame_id,
                capture_us,
                encode_us,
                convert_us,
                on_gpu,
            }),
            // The source retired its grant: the next picture opens anew.
            Ok(None) => {
                self.running = None;
                Err(unchanged())
            }
            Err(error) => {
                self.running = None;
                Err(reference.wrap(failed(ScreenErrorCode::CaptureFailed, error)))
            }
        }
    }
}
