use std::io::{self, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::Duration;

use super::limits::{MAX_WRITER_QUEUED_BYTES, WRITER_QUEUE_CAPACITY};
use super::line::MAX_JSON_LINE;
use super::Out;

pub(super) const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(super) struct Writer {
    sender: SyncSender<QueuedMessage>,
    control: Arc<WriterControl>,
    budget: Arc<QueueBudget>,
}

struct WriterControl {
    closed: AtomicBool,
    shutdown: Option<TcpStream>,
}

pub(super) struct QueuedMessage {
    json: Vec<u8>,
    _reservation: ByteReservation,
}

impl QueuedMessage {
    pub(super) fn json(&self) -> &[u8] {
        &self.json
    }

    pub(super) fn text(&self) -> io::Result<&str> {
        std::str::from_utf8(&self.json).map_err(io::Error::other)
    }
}

struct QueueBudget {
    queued: AtomicUsize,
    max: usize,
}

impl QueueBudget {
    fn try_reserve(self: &Arc<Self>, bytes: usize) -> Option<ByteReservation> {
        self.queued
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                queued.checked_add(bytes).filter(|next| *next <= self.max)
            })
            .ok()?;
        Some(ByteReservation {
            budget: self.clone(),
            bytes,
        })
    }
}

struct ByteReservation {
    budget: Arc<QueueBudget>,
    bytes: usize,
}

impl Drop for ByteReservation {
    fn drop(&mut self) {
        self.budget.queued.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

impl WriterControl {
    fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            if let Some(stream) = &self.shutdown {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

impl Writer {
    pub(super) fn websocket(stream: &TcpStream) -> io::Result<(Self, Receiver<QueuedMessage>)> {
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        Self::channel(WRITER_QUEUE_CAPACITY, Some(stream.try_clone()?))
    }

    pub(super) fn tcp(mut stream: TcpStream) -> io::Result<Self> {
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        let (writer, receiver) = Self::channel(WRITER_QUEUE_CAPACITY, Some(stream.try_clone()?))?;
        let control = writer.control.clone();
        std::thread::Builder::new()
            .name("share-server-tcp-writer".into())
            .spawn(move || {
                while !control.is_closed() {
                    let Ok(message) = receiver.recv() else {
                        break;
                    };
                    if write_tcp_message(&mut stream, &message).is_err() {
                        break;
                    }
                }
                control.close();
            })
            .map_err(io::Error::other)?;
        Ok(writer)
    }

    pub(super) fn try_send(&self, message: &Out) -> bool {
        if self.control.is_closed() {
            return false;
        }
        let json = match serialize_bounded(message) {
            Ok(json) => json,
            Err(_) => return false,
        };
        let Some(reservation) = self.budget.try_reserve(json.len()) else {
            return false;
        };
        let message = QueuedMessage {
            json,
            _reservation: reservation,
        };
        match self.sender.try_send(message) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => false,
            Err(TrySendError::Disconnected(_)) => {
                self.control.close();
                false
            }
        }
    }

    pub(super) fn close(&self) {
        self.control.close();
    }

    #[cfg(test)]
    pub(super) fn is_closed(&self) -> bool {
        self.control.is_closed()
    }

    #[cfg(test)]
    pub(super) fn queued_bytes(&self) -> usize {
        self.budget.queued.load(Ordering::Acquire)
    }

    fn channel(
        capacity: usize,
        shutdown: Option<TcpStream>,
    ) -> io::Result<(Self, Receiver<QueuedMessage>)> {
        if capacity == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "writer queue capacity must be positive",
            ));
        }
        let (sender, receiver) = mpsc::sync_channel(capacity);
        Ok((
            Self {
                sender,
                control: Arc::new(WriterControl {
                    closed: AtomicBool::new(false),
                    shutdown,
                }),
                budget: Arc::new(QueueBudget {
                    queued: AtomicUsize::new(0),
                    max: MAX_WRITER_QUEUED_BYTES,
                }),
            },
            receiver,
        ))
    }

    #[cfg(test)]
    pub(super) fn test_channel(capacity: usize) -> (Self, Receiver<Out>) {
        let (writer, queued) = Self::channel(capacity, None).unwrap();
        let (decoded_sender, decoded_receiver) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(message) = queued.recv() {
                let Ok(output) = serde_json::from_slice(message.json()) else {
                    break;
                };
                if decoded_sender.send(output).is_err() {
                    break;
                }
            }
        });
        (writer, decoded_receiver)
    }

    #[cfg(test)]
    pub(super) fn test_raw_channel(capacity: usize) -> (Self, Receiver<QueuedMessage>) {
        Self::channel(capacity, None).unwrap()
    }
}

fn serialize_bounded(message: &Out) -> io::Result<Vec<u8>> {
    let mut buffer = LimitedBuffer::new(MAX_JSON_LINE);
    serde_json::to_writer(&mut buffer, message).map_err(io::Error::other)?;
    Ok(buffer.into_inner())
}

pub(super) fn outbound_fits(message: &Out) -> bool {
    serialize_bounded(message).is_ok()
}

fn write_tcp_message(stream: &mut TcpStream, message: &QueuedMessage) -> io::Result<()> {
    stream.write_all(message.json())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

struct LimitedBuffer {
    bytes: Vec<u8>,
    max: usize,
}

impl LimitedBuffer {
    fn new(max: usize) -> Self {
        Self {
            bytes: Vec::new(),
            max,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(next_len) = self.bytes.len().checked_add(bytes.len()) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "outbound JSON message too large",
            ));
        };
        if next_len > self.max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "outbound JSON message too large",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn saturated_queue_rejects_without_blocking_and_preserves_target_connection() {
        let (writer, _receiver) = Writer::test_raw_channel(1);
        assert!(writer.try_send(&Out::Pong));

        let (result_sender, result_receiver) = mpsc::channel();
        let queued_writer = writer.clone();
        std::thread::spawn(move || {
            let _ = result_sender.send(queued_writer.try_send(&Out::Pong));
        });
        assert!(!result_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap());
        assert!(!writer.is_closed());
        assert_eq!(
            writer.queued_bytes(),
            serde_json::to_vec(&Out::Pong).unwrap().len()
        );
    }

    #[test]
    fn byte_budget_rejects_a_few_large_messages_and_releases_on_receive() {
        let (writer, receiver) = Writer::test_raw_channel(WRITER_QUEUE_CAPACITY);
        let message = Out::Error {
            scope: "test".into(),
            msg: "x".repeat(MAX_JSON_LINE - 64),
        };
        let mut accepted = 0;
        while writer.try_send(&message) {
            accepted += 1;
        }

        assert!((2..=8).contains(&accepted));
        assert!(writer.queued_bytes() <= MAX_WRITER_QUEUED_BYTES);
        assert!(!writer.is_closed());

        drop(receiver.recv().unwrap());
        assert!(writer.queued_bytes() < MAX_WRITER_QUEUED_BYTES);
    }
}
