//! One operation may run; control messages invalidate it without dropping its capture source.
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Notify;

#[derive(Default)]
pub(super) struct WorkControl {
    epoch: AtomicU64,
    reference: AtomicU64,
    changed: Notify,
}
impl WorkControl {
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
