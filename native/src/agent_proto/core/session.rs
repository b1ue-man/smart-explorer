use std::io::{self, Write};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::credit::{credit_cost, encoded_cost, RecvWindow, SendCredit};
use super::{write_frame, Frame};

/// A shared, mutex-guarded frame sink.
pub(crate) type Sink = Arc<Mutex<Box<dyn Write + Send>>>;

pub(crate) fn emit(sink: &Sink, id: u64, frame: &Frame) -> io::Result<()> {
    let mut w = sink.lock().map_err(|_| io::Error::other("sink poisoned"))?;
    write_frame(&mut *w, id, frame)
}

/// The sink of one credit-controlled request. `write_frame` hands every
/// frame to one `write` call, so the writer charges stream frames against the
/// credit the client granted before the frame takes the connection lock:
/// a request without credit waits alone, never blocking other requests.
pub(crate) fn credited_sink(shared: Sink, credit: Arc<SendCredit>) -> Sink {
    Arc::new(Mutex::new(Box::new(CreditedWriter { shared, credit })))
}

struct CreditedWriter {
    shared: Sink,
    credit: Arc<SendCredit>,
}

impl Write for CreditedWriter {
    fn write(&mut self, frame: &[u8]) -> io::Result<usize> {
        let cost = encoded_cost(frame);
        if cost > 0 {
            self.credit.take(cost, None)?;
        }
        let mut writer = self
            .shared
            .lock()
            .map_err(|_| io::Error::other("sink poisoned"))?;
        writer.write_all(frame)?;
        writer.flush()?;
        Ok(frame.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // Every frame was flushed together with its bytes in `write`.
        Ok(())
    }
}

/// Inbound stream frames of one upload request.
pub(crate) trait Inbound {
    fn recv_timeout(&self, timeout: Duration) -> Result<Frame, RecvTimeoutError>;
}

impl Inbound for Receiver<Frame> {
    fn recv_timeout(&self, timeout: Duration) -> Result<Frame, RecvTimeoutError> {
        Receiver::recv_timeout(self, timeout)
    }
}

/// Upload frames of a credit request: taking a frame returns its credit to
/// the client once half the window has been consumed.
pub(crate) struct CreditedInbound {
    pub(crate) frames: Receiver<Frame>,
    pub(crate) window: Arc<RecvWindow>,
    pub(crate) sink: Sink,
    pub(crate) id: u64,
}

impl Inbound for CreditedInbound {
    fn recv_timeout(&self, timeout: Duration) -> Result<Frame, RecvTimeoutError> {
        let frame = self.frames.recv_timeout(timeout)?;
        if let Some(bytes) = self.window.consume(credit_cost(&frame)) {
            // A lost grant only matters on a broken connection, which the
            // request loop reports on its own.
            let _ = emit(&self.sink, self.id, &Frame::Credit { bytes });
        }
        Ok(frame)
    }
}
