//! Incoming frames of one agent connection: the reader thread hands each
//! frame to the operation waiting for its request id. On a credit connection
//! every operation queue is bounded in bytes by the credit it granted, so the
//! reader never waits for a slow consumer; older servers keep the former
//! bounded queue with backpressure.
use crate::agent_proto::{credit_cost, Frame, RecvWindow, SendCredit, TRANSFER_FRAME_BACKLOG};
use crossbeam_channel::{
    bounded, unbounded, Receiver, RecvError, RecvTimeoutError, SendTimeoutError, Sender,
    TryRecvError,
};
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

pub(super) type RoutedFrame = (u64, Frame);
pub(super) type PendingMap = Arc<Mutex<HashMap<u64, Route>>>;

pub(super) fn lock_pending(pending: &PendingMap) -> MutexGuard<'_, HashMap<u64, Route>> {
    pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Flow-control state of one credit request, shared by the reader thread,
/// the consumer and the sending side.
pub(super) struct Credit {
    pub(super) window: Arc<RecvWindow>,
    pub(super) send: Arc<SendCredit>,
    /// A terminal reply arrived: the server holds nothing for this request.
    pub(super) finished: Arc<AtomicBool>,
}

pub(super) struct Route {
    pub(super) tx: Sender<Frame>,
    pub(super) credit: Option<Credit>,
    pub(super) tree_budget: Option<crate::agent_proto::TreeDecodeBudget>,
}

pub(super) fn incoming_tree_budget(
    pending: &PendingMap,
    id: u64,
) -> crate::agent_proto::TreeDecodeBudget {
    lock_pending(pending)
        .get(&id)
        .and_then(|route| route.tree_budget)
        .unwrap_or_else(|| {
            crate::agent_proto::TreeDecodeBudget::new(u64::MAX, crate::transfer::memory_budget())
        })
}

/// Replies after which the server sends nothing more for a request.
fn ends_request(frame: &Frame) -> bool {
    matches!(
        frame,
        Frame::End
            | Frame::Ok
            | Frame::Err(_)
            | Frame::Dir(_)
            | Frame::Meta(_)
            | Frame::Exists(_)
            | Frame::Tree(_)
            | Frame::HelloOk { .. }
            | Frame::Copied(_)
            | Frame::Answer(_)
            | Frame::StageDone { .. }
            | Frame::Limits(_)
    )
}

/// The receiving end of one request. Taking a frame returns its credit to
/// the server once half the window has been consumed.
pub(super) struct RequestRx {
    rx: Receiver<Frame>,
    credit: Option<RxCredit>,
}

struct RxCredit {
    id: u64,
    window: Arc<RecvWindow>,
    finished: Arc<AtomicBool>,
    control: Sender<RoutedFrame>,
}

impl RequestRx {
    /// A route of the former kind: bounded in frames.
    pub(super) fn legacy() -> (Route, Self) {
        let (tx, rx) = bounded(TRANSFER_FRAME_BACKLOG);
        (
            Route {
                tx,
                credit: None,
                tree_budget: None,
            },
            Self { rx, credit: None },
        )
    }

    /// A credit route: unbounded in frames, bounded by the granted credit.
    pub(super) fn credited(
        id: u64,
        window: Arc<RecvWindow>,
        control: Sender<RoutedFrame>,
    ) -> (Route, Self) {
        let (tx, rx) = unbounded();
        let finished = Arc::new(AtomicBool::new(false));
        let route = Route {
            tx,
            tree_budget: None,
            credit: Some(Credit {
                window: window.clone(),
                send: Arc::new(SendCredit::new()),
                finished: finished.clone(),
            }),
        };
        let rx = Self {
            rx,
            credit: Some(RxCredit {
                id,
                window,
                finished,
                control,
            }),
        };
        (route, rx)
    }

    fn took(&self, frame: &Frame) {
        let Some(credit) = &self.credit else {
            return;
        };
        if ends_request(frame) {
            credit.finished.store(true, Ordering::Release);
        }
        if let Some(bytes) = credit.window.consume(credit_cost(frame)) {
            // The control lane is unbounded; it only fails once the
            // connection is gone, which every waiter notices on its own.
            let _ = credit.control.send((credit.id, Frame::Credit { bytes }));
        }
    }

    pub(super) fn recv(&self) -> Result<Frame, RecvError> {
        let frame = self.rx.recv()?;
        self.took(&frame);
        Ok(frame)
    }

    pub(super) fn recv_timeout(&self, timeout: Duration) -> Result<Frame, RecvTimeoutError> {
        let frame = self.rx.recv_timeout(timeout)?;
        self.took(&frame);
        Ok(frame)
    }

    pub(super) fn try_recv(&self) -> Result<Frame, TryRecvError> {
        let frame = self.rx.try_recv()?;
        self.took(&frame);
        Ok(frame)
    }
}

/// Atomically close the transport and disconnect every operation waiting for a
/// response. Holding the pending lock while publishing `closed` prevents a new
/// registration from being inserted after the clear.
pub(super) fn close_transport(closed: &AtomicBool, pending: &PendingMap) {
    let mut routes = lock_pending(pending);
    closed.store(true, Ordering::Release);
    for route in routes.values() {
        if let Some(credit) = &route.credit {
            credit.send.close();
        }
    }
    routes.clear();
}

/// Route one read result; false ends the reader (transport closed).
pub(super) fn route_frame(
    pending: &PendingMap,
    activity: &super::mux::Activity,
    control: &Sender<RoutedFrame>,
    read: io::Result<Option<(u64, Frame)>>,
) -> bool {
    let Ok(Some((id, frame))) = read else {
        return false;
    };
    activity.touch();
    let (tx, credit) = {
        let routes = lock_pending(pending);
        let Some(route) = routes.get(&id) else {
            return true;
        };
        if let Frame::Credit { bytes } = frame {
            if let Some(credit) = &route.credit {
                credit.send.grant(bytes);
            }
            return true;
        }
        (
            route.tx.clone(),
            route
                .credit
                .as_ref()
                .map(|credit| (credit.window.clone(), credit.send.clone())),
        )
    };
    match credit {
        None => route_with_backpressure(pending, id, &tx, frame),
        Some((window, send)) if window.receive(credit_cost(&frame)) => {
            let ended = ends_request(&frame);
            let _ = tx.send(frame);
            if ended {
                // The server finished (or refused) the request: an upload
                // still waiting for credit stops and reads the reply.
                send.close();
            }
        }
        Some(_) => {
            // The server ignored its credit: end only this request.
            if let Some(route) = lock_pending(pending).remove(&id) {
                if let Some(credit) = route.credit {
                    credit.send.close();
                }
            }
            let _ = control.send((id, Frame::Cancel));
            let _ = tx.send(Frame::Err(
                "Gegenstelle hat das Kreditfenster der Anfrage überschritten".into(),
            ));
        }
    }
    true
}

/// Bound the queue without leaving the reader permanently parked after an
/// operation unregisters or transport teardown clears the pending map.
fn route_with_backpressure(pending: &PendingMap, id: u64, tx: &Sender<Frame>, mut frame: Frame) {
    loop {
        match tx.send_timeout(frame, Duration::from_millis(100)) {
            Ok(()) | Err(SendTimeoutError::Disconnected(_)) => return,
            Err(SendTimeoutError::Timeout(returned)) => {
                frame = returned;
                if !lock_pending(pending).contains_key(&id) {
                    return;
                }
            }
        }
    }
}
