//! Permanently active Parent reader. Resource requests never acquire the
//! serialized picture/input ask mutex, including during idle source resize.
//! An answer of the picture stream (plan 2026-11-14 „Bildkette“ B-27) comes
//! after its `STREAMED` mark and goes with it to the stream's own channel,
//! so a waiting input answer never queues behind a picture; the end of the
//! process reaches both channels.
//!
//! Plan 2026-11-17 P9: a wait for an answer ends only after its limit passed
//! without any message of the process (`progressing`): a process that still
//! asks for capture memory, reports the pointer or answers another request
//! works and is not ended for a slow first picture (encoder start). Input
//! keeps its strict limit (`INPUT_TIMEOUT`).
use super::capture_broker::CaptureBroker;
use crate::screen::core::{
    capture_transport,
    cursor_overlay::{self, CursorFeed},
    worker_capture_wire::{self as wire, GrantReply, GrantRequest, Reduction},
    worker_wire::{self, WorkerReply},
};
use std::io::{self, Read};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError},
    Arc, Mutex,
};
use std::time::Duration;
type Answer = io::Result<Option<WorkerReply>>;
/// An answer of the picture stream with its mark.
pub(super) type StreamAnswer = io::Result<Option<(worker_wire::StreamMark, WorkerReply)>>;
pub(super) struct WorkerReader {
    answers: Receiver<Answer>,
    /// Messages read from the process so far.
    activity: Arc<AtomicU64>,
}

/// The next value of `answers`; `Timeout` only after `limit` passed without
/// any message counted in `activity`.
pub(super) fn progressing<T>(
    answers: &Receiver<T>,
    activity: &AtomicU64,
    limit: Duration,
) -> Result<T, RecvTimeoutError> {
    loop {
        let seen = activity.load(Ordering::Acquire);
        match answers.recv_timeout(limit) {
            Err(RecvTimeoutError::Timeout) if activity.load(Ordering::Acquire) != seen => {}
            other => return other,
        }
    }
}
impl WorkerReader {
    pub(super) fn start(
        mut output: impl Read + Send + 'static,
        input: Arc<Mutex<std::process::ChildStdin>>,
        broker: Arc<CaptureBroker>,
        cursor: Arc<CursorFeed>,
        stream: mpsc::Sender<StreamAnswer>,
    ) -> io::Result<Self> {
        let (sender, answers) = mpsc::sync_channel(1);
        let activity = Arc::new(AtomicU64::new(0));
        let counted = activity.clone();
        std::thread::Builder::new()
            .name("screen-host-reader".into())
            .spawn(move || {
                let mut mark: Option<worker_wire::StreamMark> = None;
                loop {
                    let answer = (|| {
                        let Some((tag, length)) = capture_transport::read_header(&mut output)?
                        else {
                            return Ok(None);
                        };
                        counted.fetch_add(1, Ordering::AcqRel);
                        if tag == wire::OWNED_REFERENCE {
                            // The records go from the pipe straight into
                            // their own buffers (BK3b, no transport copy).
                            let contract = broker.contract()?;
                            return Ok(Some(Some(capture_transport::read_owned_reply(
                                &mut output,
                                length,
                                contract,
                                |id| broker.grant(id),
                            )?)));
                        }
                        let payload =
                            capture_transport::read_control(&mut output, tag, length, None)?;
                        match tag {
                            wire::GRANT_REQUEST => {
                                let request: GrantRequest =
                                    crate::screen::core::wire::from_json(&payload)?;
                                if let Some((request, approved)) = broker.resource(request) {
                                    let retryable = approved.as_ref().err().is_some_and(|error| {
                                        error.kind() == io::ErrorKind::WouldBlock
                                    });
                                    let (value, error) = match approved {
                                        Ok(value) => (Some(value), None),
                                        Err(error) => (None, Some(error.to_string())),
                                    };
                                    let payload =
                                        crate::screen::core::wire::to_json(&GrantReply {
                                            request,
                                            value,
                                            error,
                                            retryable,
                                        })?;
                                    worker_wire::write_message(
                                        &mut *input.lock().map_err(|_| {
                                            io::Error::other("worker input poisoned")
                                        })?,
                                        wire::GRANT_REPLY,
                                        &payload,
                                    )?;
                                }
                                Ok(Some(None))
                            }
                            wire::REDUCED => {
                                broker.reduced(
                                    crate::screen::core::wire::from_json::<Reduction>(&payload)?,
                                )?;
                                Ok(Some(None))
                            }
                            wire::RETIRED => {
                                broker.retired(crate::screen::core::wire::from_json(&payload)?)?;
                                Ok(Some(None))
                            }
                            // The pointer of the overlay (P6) is no answer.
                            worker_wire::CURSOR => {
                                cursor.publish(cursor_overlay::decode_update(&payload)?);
                                Ok(Some(None))
                            }
                            worker_wire::STREAMED => {
                                if mark.is_some() {
                                    return Err(io::Error::other("stream mark without answer"));
                                }
                                mark = Some(worker_wire::StreamMark::decode(&payload)?);
                                Ok(Some(None))
                            }
                            _ => Ok(Some(Some(WorkerReply::decode(tag, &payload)?))),
                        }
                    })();
                    match answer {
                        Ok(Some(None)) => continue,
                        Ok(Some(Some(reply))) => {
                            if let Some(mark) = mark.take() {
                                // Nobody waits for the stream any more: the
                                // answer goes, the link still reads.
                                let _ = stream.send(Ok(Some((mark, reply))));
                            } else if sender.send(Ok(Some(reply))).is_err() {
                                break;
                            }
                        }
                        Ok(None) => {
                            let _ = stream.send(Ok(None));
                            let _ = sender.send(Ok(None));
                            break;
                        }
                        Err(error) => {
                            let copy = io::Error::new(error.kind(), error.to_string());
                            let _ = stream.send(Err(copy));
                            let _ = sender.send(Err(error));
                            break;
                        }
                    }
                }
                broker.reader_ended();
            })?;
        Ok(Self { answers, activity })
    }
    /// The next answer; `Timeout` after `limit` without any message, or with
    /// `strict` after `limit` without the answer (input: the stream's own
    /// messages would keep a lost input answer waiting).
    pub(super) fn receive(
        &self,
        limit: Duration,
        strict: bool,
    ) -> Result<Answer, RecvTimeoutError> {
        if strict {
            return self.answers.recv_timeout(limit);
        }
        progressing(&self.answers, &self.activity, limit)
    }
    /// The message count of the process, for the picture stream's wait.
    pub(super) fn activity(&self) -> Arc<AtomicU64> {
        self.activity.clone()
    }
}
