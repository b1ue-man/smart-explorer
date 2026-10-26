//! One response budget per serialized worker request. The pipe reader must not
//! snapshot a previous request's limits before waiting for the next response.
use std::io::{self, Read};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::time::Duration;

use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::worker_wire::read_message_with_limits;

type Answer = io::Result<Option<(u8, Vec<u8>)>>;

pub(super) struct WorkerReader {
    requests: SyncSender<Option<FrameLimits>>,
    answers: Receiver<Answer>,
}

impl WorkerReader {
    pub(super) fn start(mut output: impl Read + Send + 'static) -> io::Result<Self> {
        let (requests, budgets) = mpsc::sync_channel(1);
        let (sender, answers) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("screen-host-reader".into())
            .spawn(move || {
                for limits in budgets {
                    let answer = read_message_with_limits(&mut output, limits);
                    let end = !matches!(answer, Ok(Some(_)));
                    if sender.send(answer).is_err() || end {
                        return;
                    }
                }
            })?;
        Ok(Self { requests, answers })
    }

    pub(super) fn begin(&self, limits: Option<FrameLimits>) -> io::Result<()> {
        self.requests.try_send(limits).map_err(|error| {
            let (kind, detail) = match error {
                TrySendError::Full(_) => (io::ErrorKind::WouldBlock, "screen host reader still owns a prior request"),
                TrySendError::Disconnected(_) => (io::ErrorKind::BrokenPipe, "screen host reader ended"),
            };
            io::Error::new(kind, detail)
        })
    }

    pub(super) fn receive(&self, timeout: Duration) -> Result<Answer, RecvTimeoutError> {
        self.answers.recv_timeout(timeout)
    }
}
