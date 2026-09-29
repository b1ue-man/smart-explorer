//! Per-connection state of a request server (the agent and the background
//! service share it): cancellation, upload routing and, once the client
//! switched the connection to credit mode, per-request flow control.
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};

use super::credit::CREDIT_REQUEST_LIMIT;
use super::credit::{busy_message, credit_cost, RecvWindow, SendCredit, StreamCount};
use super::session::{credited_sink, emit, CreditedInbound, Inbound, Sink};
use super::{Frame, TRANSFER_FRAME_BACKLOG};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

enum UploadSender {
    /// Former behaviour: a bounded queue that holds the reader back.
    Bounded(SyncSender<Frame>),
    /// Credit mode: bounded in bytes by the granted credit, never blocking.
    Credited(Sender<Frame>, Arc<RecvWindow>),
}

/// What a request worker needs: its own sink, its upload frames and its
/// cancellation flag.
pub(crate) struct RequestContext {
    pub(crate) sink: Sink,
    pub(crate) inbound: Option<Box<dyn Inbound + Send>>,
    pub(crate) cancel: Arc<AtomicBool>,
}

pub(crate) struct ServerSession {
    sink: Sink,
    credit_mode: AtomicBool,
    uploads: Mutex<HashMap<u64, UploadSender>>,
    cancels: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    credits: Mutex<HashMap<u64, Arc<SendCredit>>>,
    streams: Arc<StreamCount>,
}

/// Upload queue of a connection without credit: bounded in frames, so a
/// full queue holds the connection reader back (the former behaviour).
pub(crate) fn transfer_channel() -> (SyncSender<Frame>, Receiver<Frame>) {
    sync_channel(TRANSFER_FRAME_BACKLOG)
}

/// Requests whose client streams frames after the request frame.
pub(crate) fn is_upload(request: &Frame) -> bool {
    matches!(
        request,
        Frame::Write(_) | Frame::WriteNew(_) | Frame::PutTree(_) | Frame::BatchPut { .. }
    )
}

impl ServerSession {
    pub(crate) fn new(sink: Sink) -> Arc<Self> {
        Arc::new(Self {
            sink,
            credit_mode: AtomicBool::new(false),
            uploads: Mutex::new(HashMap::new()),
            cancels: Mutex::new(HashMap::new()),
            credits: Mutex::new(HashMap::new()),
            streams: Arc::new(StreamCount::default()),
        })
    }

    pub(crate) fn sink(&self) -> &Sink {
        &self.sink
    }

    pub(crate) fn credit_mode(&self) -> bool {
        self.credit_mode.load(Ordering::Acquire)
    }

    /// Concurrent requests this connection admits (`legacy` without credit).
    pub(crate) fn request_limit(&self, legacy: usize) -> usize {
        if self.credit_mode() {
            CREDIT_REQUEST_LIMIT
        } else {
            legacy
        }
    }

    /// Consume stream, credit and cancel frames; hand back a frame that
    /// starts a new request.
    pub(crate) fn route(&self, id: u64, frame: Frame) -> Option<Frame> {
        match frame {
            Frame::Data(_) | Frame::TreeEntry { .. } | Frame::ItemEnd { .. } | Frame::End => {
                self.route_upload(id, frame);
                None
            }
            Frame::Cancel => {
                self.cancel(id);
                None
            }
            Frame::Credit { bytes } => {
                if id == 0 {
                    self.credit_mode.store(true, Ordering::Release);
                } else if let Some(credit) = lock(&self.credits).get(&id) {
                    credit.grant(bytes);
                }
                None
            }
            request => Some(request),
        }
    }

    fn route_upload(&self, id: u64, frame: Frame) {
        let is_end = matches!(frame, Frame::End);
        let mut uploads = lock(&self.uploads);
        let violated = match uploads.get(&id) {
            None => return,
            Some(UploadSender::Credited(sender, window)) => {
                if window.receive(credit_cost(&frame)) {
                    let _ = sender.send(frame);
                    false
                } else {
                    true
                }
            }
            Some(UploadSender::Bounded(sender)) => {
                let sender = sender.clone();
                drop(uploads);
                // Former behaviour for clients without credit: the queue
                // holds the connection reader back while it is full.
                let _ = sender.send(frame);
                if is_end {
                    lock(&self.uploads).remove(&id);
                }
                return;
            }
        };
        if violated || is_end {
            uploads.remove(&id);
        }
        drop(uploads);
        if violated {
            // The client exceeded its credit: end only this request.
            self.cancel(id);
        }
    }

    /// Whether `id` still runs (a new request must use a fresh id).
    pub(crate) fn is_active(&self, id: u64) -> bool {
        lock(&self.cancels).contains_key(&id)
    }

    /// Register a new request and build its worker context.
    pub(crate) fn open(&self, id: u64, request: &Frame) -> RequestContext {
        let cancel = Arc::new(AtomicBool::new(false));
        lock(&self.cancels).insert(id, cancel.clone());
        let credit_mode = self.credit_mode();
        let sink = if credit_mode {
            let credit = Arc::new(SendCredit::new());
            lock(&self.credits).insert(id, credit.clone());
            credited_sink(self.sink.clone(), credit)
        } else {
            self.sink.clone()
        };
        let inbound = is_upload(request).then(|| self.open_upload(id, credit_mode));
        RequestContext {
            sink,
            inbound,
            cancel,
        }
    }

    fn open_upload(&self, id: u64, credit_mode: bool) -> Box<dyn Inbound + Send> {
        if credit_mode {
            let (sender, frames) = channel();
            let window = Arc::new(RecvWindow::new(self.streams.clone()));
            lock(&self.uploads).insert(id, UploadSender::Credited(sender, window.clone()));
            Box::new(CreditedInbound {
                frames,
                window,
                sink: self.sink.clone(),
                id,
            })
        } else {
            let (sender, frames) = transfer_channel();
            lock(&self.uploads).insert(id, UploadSender::Bounded(sender));
            Box::new(frames)
        }
    }

    /// A worker finished its request.
    pub(crate) fn close(&self, id: u64) {
        lock(&self.cancels).remove(&id);
        lock(&self.uploads).remove(&id);
        if let Some(credit) = lock(&self.credits).remove(&id) {
            credit.close();
        }
    }

    /// Cancel one request: its handler stops at the next check, a blocked
    /// upload receive or credit wait wakes up.
    pub(crate) fn cancel(&self, id: u64) {
        if let Some(cancel) = lock(&self.cancels).get(&id) {
            cancel.store(true, Ordering::Relaxed);
        }
        // Dropping the final sender wakes handlers currently blocked in recv.
        lock(&self.uploads).remove(&id);
        if let Some(credit) = lock(&self.credits).get(&id) {
            credit.close();
        }
    }

    /// The connection ended: cancel every request.
    pub(crate) fn abort_all(&self) {
        for cancel in lock(&self.cancels).values() {
            cancel.store(true, Ordering::Relaxed);
        }
        lock(&self.uploads).clear();
        for credit in lock(&self.credits).values() {
            credit.close();
        }
    }

    /// Refuse a request because the connection is at its limit; a credit
    /// client reads the marker as congestion.
    pub(crate) fn reject_busy(&self, id: u64, legacy_text: &str) -> io::Result<()> {
        let text = if self.credit_mode() {
            busy_message(None, legacy_text)
        } else {
            legacy_text.to_string()
        };
        emit(&self.sink, id, &Frame::Err(text))
    }

    /// Forget a request whose worker could not start.
    pub(crate) fn discard(&self, id: u64) {
        self.close(id);
    }
}
