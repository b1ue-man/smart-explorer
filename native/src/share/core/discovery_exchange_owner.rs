//! In-process exchange ownership, independent of any platform lock type.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The daemon connection owns this cancellation handle. It does not retain
/// the operation lease after an ambiguous command acknowledgement.
pub(crate) struct DiscoveryExchangeOwner(Arc<AtomicBool>);

#[derive(Clone)]
pub(crate) struct DiscoveryExchangeCancellation(Arc<AtomicBool>);

/// Only the pending command and the active exchange retain this marker.
/// A cloned command shares its marker, so dropping one copy cannot release
/// the operation while a different copy is still in the worker.
#[derive(Clone)]
pub struct DiscoveryExchangeLease(Arc<LeaseState>);

struct LeaseState {
    alive: Arc<AtomicBool>,
    // Opaque to core: the daemon supplies an already admitted operation.
    // The last pending/active reference releases it, including on errors.
    _retainer: Option<Arc<dyn Send + Sync>>,
}

impl DiscoveryExchangeOwner {
    pub(crate) fn new(retainer: Option<Arc<dyn Send + Sync>>) -> (Self, DiscoveryExchangeLease) {
        let alive = Arc::new(AtomicBool::new(true));
        let lease = DiscoveryExchangeLease(Arc::new(LeaseState {
            alive: Arc::clone(&alive),
            _retainer: retainer,
        }));
        (Self(alive), lease)
    }

    pub(crate) fn cancellation(&self) -> DiscoveryExchangeCancellation {
        DiscoveryExchangeCancellation(Arc::clone(&self.0))
    }
}

impl Drop for DiscoveryExchangeOwner {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl DiscoveryExchangeCancellation {
    pub(crate) fn cancel(&self) {
        self.0.store(false, Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        !self.0.load(Ordering::Acquire)
    }
}

impl DiscoveryExchangeLease {
    pub(super) fn is_cancelled(&self) -> bool {
        !self.0.alive.load(Ordering::Acquire)
    }
}

impl std::fmt::Debug for DiscoveryExchangeLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DiscoveryExchangeLease { owner: [IN-PROCESS] }")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Retention(Arc<AtomicBool>);

    impl Drop for Retention {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[test]
    fn batches_task_p29_operation_stays_until_last_pending_or_active_lease() {
        let released = Arc::new(AtomicBool::new(false));
        let retainer: Arc<dyn Send + Sync> = Arc::new(Retention(Arc::clone(&released)));
        let (owner, pending) = DiscoveryExchangeOwner::new(Some(retainer));
        let active = pending.clone();
        assert!(!active.is_cancelled());
        drop(owner);
        assert!(active.is_cancelled());
        assert!(!released.load(Ordering::Acquire));
        drop(pending);
        assert!(!released.load(Ordering::Acquire));
        drop(active);
        assert!(released.load(Ordering::Acquire));
    }

    #[test]
    fn batches_task_p29_owned_start_cannot_be_supplied_or_transmitted_by_ipc() {
        use crate::share::{DiscoveryCommand, DiscoveryPin};
        let (_owner, lease) = DiscoveryExchangeOwner::new(None);
        let command = DiscoveryCommand::StartOwnedDiscoveryExchange {
            discovery_id: "offer".into(),
            pin: DiscoveryPin::new("private-input-sentinel".into()),
            share_back: false,
            owner: lease,
        };
        assert!(serde_json::to_string(&command).is_err());
        assert!(serde_json::from_str::<DiscoveryCommand>(
            r#"{"StartOwnedDiscoveryExchange":{"discovery_id":"offer","pin":"value","share_back":false}}"#
        ).is_err());
        assert!(!format!("{command:?}").contains("private-input-sentinel"));
    }
}
