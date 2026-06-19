use crate::agent_proto::Frame;
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Bound on un-sent outgoing frames. Provides backpressure for uploads while
/// still pipelining roughly 8 MiB of 256 KiB chunks ahead of the wire.
pub(super) const OUT_BACKLOG: usize = 32;

/// Shared multiplexer over one agent channel.
pub(super) struct Mux {
    /// Outgoing frames to the writer thread. FIFO preserves per-op ordering.
    pub(super) out: Sender<(u64, Frame)>,
    /// req_id to the op waiting for its reply/stream frames.
    pub(super) pending: Arc<Mutex<HashMap<u64, Sender<Frame>>>>,
    pub(super) next_id: AtomicU64,
}

impl Mux {
    pub(super) fn new(out: Sender<(u64, Frame)>, pending: Arc<Mutex<HashMap<u64, Sender<Frame>>>>) -> Self {
        Self { out, pending, next_id: AtomicU64::new(1) }
    }

    /// Allocate a fresh req_id and a channel to receive its frames.
    pub(super) fn register(&self) -> (u64, Receiver<Frame>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = unbounded();
        if let Ok(mut p) = self.pending.lock() {
            p.insert(id, tx);
        }
        (id, rx)
    }

    pub(super) fn unregister(&self, id: u64) {
        if let Ok(mut p) = self.pending.lock() {
            p.remove(&id);
        }
    }

    pub(super) fn send(&self, id: u64, frame: Frame) -> io::Result<()> {
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

pub(super) fn make_out_channel() -> (Sender<(u64, Frame)>, Receiver<(u64, Frame)>) {
    bounded::<(u64, Frame)>(OUT_BACKLOG)
}

pub(super) fn route_frame(
    pending: &Arc<Mutex<HashMap<u64, Sender<Frame>>>>,
    read: io::Result<Option<(u64, Frame)>>,
) -> bool {
    match read {
        Ok(Some((id, frame))) => {
            let tx = pending.lock().ok().and_then(|p| p.get(&id).cloned());
            if let Some(tx) = tx {
                let _ = tx.send(frame);
            }
            true
        }
        _ => false,
    }
}
