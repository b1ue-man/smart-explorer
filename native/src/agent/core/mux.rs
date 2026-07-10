use crate::agent_proto::{Frame, TRANSFER_FRAME_BACKLOG};
use crossbeam_channel::{bounded, Receiver, SendTimeoutError, Sender};
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

type RoutedFrame = (u64, Frame);
type PendingMap = Arc<Mutex<HashMap<u64, Sender<Frame>>>>;

/// Bound on un-sent outgoing frames. Provides backpressure for uploads while
/// still pipelining roughly 8 MiB of 256 KiB chunks ahead of the wire.
pub(super) const OUT_BACKLOG: usize = 32;

/// Shared multiplexer over one agent channel.
pub(super) struct Mux {
    /// Outgoing frames to the writer thread. FIFO preserves per-op ordering.
    pub(super) out: Sender<RoutedFrame>,
    /// req_id to the op waiting for its reply/stream frames.
    pub(super) pending: PendingMap,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) next_id: AtomicU64,
}

impl Mux {
    pub(super) fn new(
        out: Sender<RoutedFrame>,
        pending: PendingMap,
        closed: Arc<AtomicBool>,
    ) -> Self {
        Self {
            out,
            pending,
            closed,
            next_id: AtomicU64::new(1),
        }
    }

    /// Allocate a fresh req_id and a channel to receive its frames.
    pub(super) fn register(&self) -> (u64, Receiver<Frame>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = bounded(TRANSFER_FRAME_BACKLOG);
        let mut p = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.closed.load(Ordering::Acquire) {
            p.insert(id, tx);
        }
        (id, rx)
    }

    pub(super) fn unregister(&self, id: u64) {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&id);
    }

    pub(super) fn send(&self, id: u64, frame: Frame) -> io::Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "agent transport closed",
            ));
        }
        self.out
            .send((id, frame))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "agent writer gone"))
    }

    /// One request to one response frame. Registers, sends, waits for the first
    /// frame, then unregisters.
    pub(super) fn call(&self, req: Frame) -> io::Result<Frame> {
        let (id, rx) = self.register();
        let r = (|| {
            self.send(id, req)?;
            rx.recv()
                .map_err(|_| io::Error::new(io::ErrorKind::UnexpectedEof, "agent stream closed"))
        })();
        self.unregister(id);
        r
    }
}

pub(super) fn make_out_channel() -> (Sender<RoutedFrame>, Receiver<RoutedFrame>) {
    bounded::<RoutedFrame>(OUT_BACKLOG)
}

/// Atomically close the transport and disconnect every operation waiting for a
/// response. Holding the pending lock while publishing `closed` prevents a new
/// registration from being inserted after the clear.
pub(super) fn close_transport(closed: &AtomicBool, pending: &PendingMap) {
    let mut p = pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    closed.store(true, Ordering::Release);
    p.clear();
}

pub(super) fn route_frame(pending: &PendingMap, read: io::Result<Option<(u64, Frame)>>) -> bool {
    match read {
        Ok(Some((id, frame))) => {
            let tx = pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&id)
                .cloned();
            if let Some(tx) = tx {
                route_with_backpressure(pending, id, &tx, frame);
            }
            true
        }
        _ => false,
    }
}

/// Bound the queue without leaving the reader permanently parked after an
/// operation unregisters or transport teardown clears the pending map.
fn route_with_backpressure(pending: &PendingMap, id: u64, tx: &Sender<Frame>, mut frame: Frame) {
    loop {
        match tx.send_timeout(frame, Duration::from_millis(100)) {
            Ok(()) | Err(SendTimeoutError::Disconnected(_)) => return,
            Err(SendTimeoutError::Timeout(returned)) => {
                frame = returned;
                if !pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .contains_key(&id)
                {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{close_transport, make_out_channel, route_frame, Mux};
    use crate::agent_proto::{Frame, TRANSFER_FRAME_BACKLOG};
    use crossbeam_channel::TrySendError;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[test]
    fn closing_transport_disconnects_existing_and_new_requests() {
        let (out, _out_rx) = make_out_channel();
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));
        let mux = Mux::new(out, pending.clone(), closed.clone());

        let (_, existing) = mux.register();
        close_transport(&closed, &pending);
        assert!(existing.recv().is_err());

        let (id, after_close) = mux.register();
        assert!(after_close.recv().is_err());
        assert!(mux.send(id, Frame::Ok).is_err());
    }

    #[test]
    fn registered_stream_is_bounded_and_receiver_drop_unblocks_sender() {
        let (out, _out_rx) = make_out_channel();
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));
        let mux = Mux::new(out, pending.clone(), closed);

        let (id, receiver) = mux.register();
        let sender = pending.lock().unwrap().get(&id).unwrap().clone();
        for _ in 0..TRANSFER_FRAME_BACKLOG {
            sender.try_send(Frame::Ok).unwrap();
        }
        assert!(matches!(
            sender.try_send(Frame::Ok),
            Err(TrySendError::Full(Frame::Ok))
        ));

        drop(receiver);
        assert!(matches!(
            sender.try_send(Frame::Ok),
            Err(TrySendError::Disconnected(Frame::Ok))
        ));
    }

    #[test]
    fn unregister_releases_a_router_waiting_on_backpressure() {
        let (out, _out_rx) = make_out_channel();
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));
        let mux = Mux::new(out, pending.clone(), closed);
        let (id, receiver) = mux.register();
        let sender = pending.lock().unwrap().get(&id).unwrap().clone();
        for _ in 0..TRANSFER_FRAME_BACKLOG {
            sender.try_send(Frame::Ok).unwrap();
        }
        drop(sender);

        let routed = pending.clone();
        let router = std::thread::spawn(move || route_frame(&routed, Ok(Some((id, Frame::Ok)))));
        mux.unregister(id);
        assert!(router.join().unwrap());

        for _ in 0..TRANSFER_FRAME_BACKLOG {
            assert_eq!(receiver.recv().unwrap(), Frame::Ok);
        }
        assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());
    }
}
