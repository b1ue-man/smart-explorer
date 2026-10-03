use super::lanes::{is_data_frame, OutLanes};
use super::route::{close_transport, lock_pending, PendingMap, RequestRx, RoutedFrame};
use crate::agent_proto::{credit_cost, Frame, RecvWindow, SendCredit, StreamCount};
use crossbeam_channel::{RecvTimeoutError, SendTimeoutError};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Shared multiplexer over one agent channel.
pub(super) struct Mux {
    /// Outgoing frames to the writer thread; each lane is FIFO, so every
    /// operation's frames keep their order.
    pub(super) out: OutLanes,
    /// req_id to the op waiting for its reply/stream frames.
    pub(super) pending: PendingMap,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) next_id: AtomicU64,
    pub(super) link_aware_hash: AtomicBool,
    /// The server announced `credit-v1` and the connection switched to it.
    credit: AtomicBool,
    streams: Arc<StreamCount>,
    retired: AtomicBool,
    activity: Arc<Activity>,
    stall_timeout: Duration,
}

pub(super) struct Activity {
    last: Mutex<Instant>,
}

impl Activity {
    fn new() -> Self {
        Self {
            last: Mutex::new(Instant::now()),
        }
    }

    pub(super) fn touch(&self) {
        *self
            .last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Instant::now();
    }

    fn idle_for(&self) -> Duration {
        self.last_activity().elapsed()
    }

    fn last_activity(&self) -> Instant {
        *self
            .last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Mux {
    pub(super) fn new_with_stall_timeout(
        out: OutLanes,
        pending: PendingMap,
        closed: Arc<AtomicBool>,
        stall_timeout: Duration,
    ) -> Self {
        Self {
            out,
            pending,
            closed,
            next_id: AtomicU64::new(1),
            link_aware_hash: AtomicBool::new(false),
            credit: AtomicBool::new(false),
            streams: Arc::new(StreamCount::default()),
            retired: AtomicBool::new(false),
            activity: Arc::new(Activity::new()),
            stall_timeout,
        }
    }

    /// Switch the connection to credit flow control. Called once, right
    /// after the handshake and before any other request, so the server sees
    /// the switch (request id 0) before the first credit request.
    pub(super) fn enable_credit(&self) -> io::Result<()> {
        self.out
            .control
            .send((0, Frame::Credit { bytes: 0 }))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "agent writer gone"))?;
        self.credit.store(true, Ordering::Release);
        Ok(())
    }

    pub(super) fn credit_mode(&self) -> bool {
        self.credit.load(Ordering::Acquire)
    }

    /// Allocate a fresh req_id and a channel to receive its frames.
    pub(super) fn register(&self) -> (u64, RequestRx) {
        self.register_with_tree_budget(None)
    }

    pub(super) fn register_with_tree_budget(&self, budget: Option<crate::agent_proto::TreeDecodeBudget>) -> (u64, RequestRx) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (mut route, rx) = if self.credit_mode() {
            RequestRx::credited(
                id,
                Arc::new(RecvWindow::new(self.streams.clone())),
                self.out.control.clone(),
            )
        } else {
            RequestRx::legacy()
        };
        route.tree_budget = budget;
        let mut p = lock_pending(&self.pending);
        if !self.closed.load(Ordering::Acquire) && !self.retired.load(Ordering::Acquire) {
            p.insert(id, route);
        }
        (id, rx)
    }

    /// Forget a request. A credit request the server may still be serving
    /// is canceled, so the server never waits for credit that cannot come.
    pub(super) fn unregister(&self, id: u64) {
        let mut pending = lock_pending(&self.pending);
        if let Some(route) = pending.remove(&id) {
            if let Some(credit) = route.credit {
                credit.send.close();
                if !credit.finished.load(Ordering::Acquire) {
                    let _ = self.out.control.send((id, Frame::Cancel));
                }
            }
        }
        if pending.is_empty() && self.retired.load(Ordering::Acquire) {
            self.closed.store(true, Ordering::Release);
        }
    }

    fn send_credit(&self, id: u64) -> Option<Arc<SendCredit>> {
        lock_pending(&self.pending)
            .get(&id)
            .and_then(|route| route.credit.as_ref().map(|credit| credit.send.clone()))
    }

    pub(super) fn send(&self, id: u64, frame: Frame) -> io::Result<()> {
        self.ensure_request_active(id)?;
        if !is_data_frame(&frame) {
            return self.send_control((id, frame));
        }
        let cost = credit_cost(&frame);
        if cost > 0 {
            if let Some(credit) = self.send_credit(id) {
                // The server grants as it consumes; a server that takes
                // nothing for a whole stall period fails only this request.
                credit.take(cost, Some(self.stall_timeout))?;
            }
        }
        let mut routed = (id, frame);
        let mut last_activity = self.activity.last_activity();
        let mut deadline = Instant::now() + self.stall_timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                self.close();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "agent writer queue stalled",
                ));
            }
            match self.out.data.send_timeout(routed, deadline - now) {
                Ok(()) => return Ok(()),
                Err(SendTimeoutError::Disconnected(_)) => {
                    self.close();
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "agent writer gone",
                    ));
                }
                Err(SendTimeoutError::Timeout(returned)) => {
                    routed = returned;
                    let latest = self.activity.last_activity();
                    if latest > last_activity {
                        last_activity = latest;
                        deadline = latest + self.stall_timeout;
                    }
                }
            }
        }
    }

    fn send_control(&self, routed: RoutedFrame) -> io::Result<()> {
        self.out.control.send(routed).map_err(|_| {
            self.close();
            io::Error::new(io::ErrorKind::BrokenPipe, "agent writer gone")
        })
    }

    pub(super) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub(super) fn is_retired(&self) -> bool {
        self.retired.load(Ordering::Acquire)
    }

    /// Stop accepting new requests while allowing every already-registered
    /// stream to drain. The final unregister closes the generation.
    pub(super) fn retire(&self) {
        let pending = lock_pending(&self.pending);
        self.retired.store(true, Ordering::Release);
        if pending.is_empty() {
            self.closed.store(true, Ordering::Release);
        }
    }

    pub(super) fn close(&self) {
        close_transport(&self.closed, &self.pending);
    }

    pub(super) fn activity(&self) -> Arc<Activity> {
        self.activity.clone()
    }

    pub(super) fn idle_for(&self) -> Duration {
        self.activity.idle_for()
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

    /// One request with a fixed wall-clock deadline. Unlike the streaming
    /// inactivity timeout, unrelated frames on this transport never extend
    /// the request's lifetime.
    pub(super) fn call_absolute_timeout(&self, req: Frame, timeout: Duration) -> io::Result<Frame> {
        let deadline = Instant::now() + timeout;
        let (id, rx) = self.register();
        let result = (|| {
            self.ensure_request_active(id)?;
            // Requests take the unbounded control lane: nothing to wait for.
            self.send_control((id, req))?;
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(frame) => Ok(frame),
                Err(RecvTimeoutError::Timeout) => Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "agent request absolute timeout",
                )),
                Err(RecvTimeoutError::Disconnected) => Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "agent stream closed",
                )),
            }
        })();
        self.unregister(id);
        result
    }

    fn ensure_request_active(&self, id: u64) -> io::Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "agent transport closed",
            ));
        }
        if self.retired.load(Ordering::Acquire) && !lock_pending(&self.pending).contains_key(&id) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "agent transport generation retired",
            ));
        }
        Ok(())
    }

    pub(super) fn call_inactivity_timeout(
        &self,
        req: Frame,
        timeout: Duration,
    ) -> io::Result<Frame> {
        let (id, rx) = self.register();
        let result = (|| {
            self.send(id, req)?;
            let mut last_activity = self.activity.last_activity();
            let mut deadline = Instant::now() + timeout;
            loop {
                let latest = self.activity.last_activity();
                if latest > last_activity {
                    last_activity = latest;
                    deadline = latest + timeout;
                }
                let now = Instant::now();
                if now >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "agent response inactivity timeout",
                    ));
                }
                match rx.recv_timeout(deadline - now) {
                    Ok(frame) => return Ok(frame),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "agent stream closed",
                        ));
                    }
                }
            }
        })();
        self.unregister(id);
        result
    }
}

#[cfg(test)]
#[path = "mux_tests.rs"]
mod tests;
