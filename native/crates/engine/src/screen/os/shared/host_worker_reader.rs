//! Permanently active Parent reader. Resource requests never acquire the
//! serialized picture/input ask mutex, including during idle source resize.
use super::capture_broker::CaptureBroker;
use crate::screen::core::{
    capture_transport,
    worker_capture_wire::{self as wire, GrantReply, GrantRequest, Reduction},
    worker_wire::{self, WorkerReply},
};
use std::io::{self, Read};
use std::sync::{
    mpsc::{self, Receiver, RecvTimeoutError},
    Arc, Mutex,
};
use std::time::Duration;
type Answer = io::Result<Option<WorkerReply>>;
pub(super) struct WorkerReader {
    answers: Receiver<Answer>,
}
impl WorkerReader {
    pub(super) fn start(
        mut output: impl Read + Send + 'static,
        input: Arc<Mutex<std::process::ChildStdin>>,
        broker: Arc<CaptureBroker>,
    ) -> io::Result<Self> {
        let (sender, answers) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("screen-host-reader".into())
            .spawn(move || {
                loop {
                    let answer = (|| {
                        let Some((tag, length)) = capture_transport::read_header(&mut output)?
                        else {
                            return Ok(None);
                        };
                        if tag == wire::OWNED_REFERENCE {
                            let contract = broker.contract()?;
                            let (_, grant, payload) = capture_transport::read_owned(
                                &mut output,
                                length,
                                contract,
                                |id| broker.grant(id),
                            )?;
                            return Ok(Some(Some(capture_transport::decode_owned(
                                &payload, &grant, contract,
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
                            _ => Ok(Some(Some(WorkerReply::decode(tag, &payload)?))),
                        }
                    })();
                    match answer {
                        Ok(Some(None)) => continue,
                        Ok(Some(Some(reply))) => {
                            if sender.send(Ok(Some(reply))).is_err() {
                                break;
                            }
                        }
                        Ok(None) => {
                            let _ = sender.send(Ok(None));
                            break;
                        }
                        Err(error) => {
                            let _ = sender.send(Err(error));
                            break;
                        }
                    }
                }
                broker.reader_ended();
            })?;
        Ok(Self { answers })
    }
    pub(super) fn receive(&self, timeout: Duration) -> Result<Answer, RecvTimeoutError> {
        self.answers.recv_timeout(timeout)
    }
}
