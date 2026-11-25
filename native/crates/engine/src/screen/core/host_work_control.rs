//! One operation may run; control messages invalidate it without dropping its capture source.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

use crate::screen::core::video_transport::video_route::VideoRoute;

#[derive(Default)]
pub(super) struct WorkControl {
    epoch: AtomicU64,
    reference: AtomicU64,
    changed: Notify,
    /// „Höchste Datenrate“ as it is now (0: none), read by a picture's send
    /// between its pieces (Codex 5/12: a lowered ceiling waited for the
    /// running picture).
    ceiling: Arc<AtomicU64>,
    /// The picture datagrams' state the session shares (`video_route.rs`).
    video: Arc<VideoRoute>,
}
impl WorkControl {
    pub(super) fn video(&self) -> &Arc<VideoRoute> {
        &self.video
    }
    pub(super) fn set_live_ceiling(&self, ceiling: Option<u64>) {
        self.ceiling.store(ceiling.unwrap_or(0), Ordering::Release);
    }
    pub(super) fn ceiling_cell(&self) -> Arc<AtomicU64> {
        self.ceiling.clone()
    }
    pub(super) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }
    pub(super) fn reference(&self) -> u64 {
        self.reference.load(Ordering::Acquire)
    }
    pub(super) fn invalidate_reference(&self) {
        self.reference.fetch_add(1, Ordering::AcqRel);
    }
    pub(super) fn interrupt(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.invalidate_reference();
        self.changed.notify_waiters();
    }
    pub(super) fn current(&self, epoch: u64) -> bool {
        self.epoch() == epoch
    }
    pub(super) async fn cancelled(&self, epoch: u64) {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self.current(epoch) {
                return;
            }
            notified.await;
        }
    }
}
