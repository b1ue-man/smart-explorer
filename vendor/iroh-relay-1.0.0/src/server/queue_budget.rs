//! Byte budgets for packets retained by relay client send queues.

use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// A single slow recipient may retain at most sixteen maximum-sized packets.
const PER_CLIENT_OUTGOING_PACKET_BYTES: usize = 1024 * 1024;
/// All client packet queues together may retain at most 64 MiB of payload data.
const GLOBAL_OUTGOING_PACKET_BYTES: usize = 64 * 1024 * 1024;

/// Factory for per-client packet budgets sharing one relay-wide ceiling.
#[derive(Debug, Clone)]
pub(super) struct PacketQueueBudget {
    global: Arc<Semaphore>,
    per_client_bytes: usize,
}

impl Default for PacketQueueBudget {
    fn default() -> Self {
        Self::new(
            GLOBAL_OUTGOING_PACKET_BYTES,
            PER_CLIENT_OUTGOING_PACKET_BYTES,
        )
    }
}

impl PacketQueueBudget {
    pub(super) fn new(global_bytes: usize, per_client_bytes: usize) -> Self {
        debug_assert!(global_bytes <= Semaphore::MAX_PERMITS);
        debug_assert!(per_client_bytes <= u32::MAX as usize);
        Self {
            global: Arc::new(Semaphore::new(global_bytes)),
            per_client_bytes,
        }
    }

    pub(super) fn client(&self) -> ClientPacketQueueBudget {
        ClientPacketQueueBudget {
            global: self.global.clone(),
            local: Arc::new(Semaphore::new(self.per_client_bytes)),
        }
    }

    #[cfg(test)]
    pub(super) fn available_global_bytes(&self) -> usize {
        self.global.available_permits()
    }
}

/// Byte budget used by one client's outgoing packet queue.
#[derive(Debug, Clone)]
pub(super) struct ClientPacketQueueBudget {
    global: Arc<Semaphore>,
    local: Arc<Semaphore>,
}

impl ClientPacketQueueBudget {
    pub(super) fn try_reserve(&self, bytes: usize) -> Option<PacketQueuePermit> {
        // Empty datagrams are rejected by the protocol writer, but still charge one byte
        // so malformed internal callers cannot create unaccounted queue entries.
        let bytes = bytes.max(1);
        let permits = u32::try_from(bytes).ok()?;
        let global = self.global.clone().try_acquire_many_owned(permits).ok()?;
        let local = self.local.clone().try_acquire_many_owned(permits).ok()?;
        Some(PacketQueuePermit {
            _global: global,
            _local: local,
        })
    }

    #[cfg(test)]
    pub(super) fn available_local_bytes(&self) -> usize {
        self.local.available_permits()
    }
}

/// RAII reservation retained by a queued packet until its write attempt completes.
#[derive(Debug)]
pub(super) struct PacketQueuePermit {
    _global: OwnedSemaphorePermit,
    _local: OwnedSemaphorePermit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_client_saturation_releases_global_reservation_on_failure() {
        let budget = PacketQueueBudget::new(12, 8);
        let first_client = budget.client();
        let second_client = budget.client();
        let first = first_client.try_reserve(8).expect("first reservation");

        assert!(first_client.try_reserve(1).is_none());
        assert_eq!(budget.available_global_bytes(), 4);
        let second = second_client
            .try_reserve(4)
            .expect("failed local reservation did not leak global bytes");
        assert_eq!(budget.available_global_bytes(), 0);

        drop(second);
        drop(first);
        assert_eq!(budget.available_global_bytes(), 12);
        assert_eq!(first_client.available_local_bytes(), 8);
    }

    #[test]
    fn global_saturation_is_shared_and_capacity_is_reusable() {
        let budget = PacketQueueBudget::new(10, 10);
        let first_client = budget.client();
        let second_client = budget.client();
        let first = first_client.try_reserve(6).expect("first reservation");
        let second = second_client.try_reserve(4).expect("second reservation");

        assert!(second_client.try_reserve(1).is_none());
        assert_eq!(budget.available_global_bytes(), 0);

        drop(first);
        let replacement = second_client
            .try_reserve(6)
            .expect("released global capacity is reusable");
        assert_eq!(budget.available_global_bytes(), 0);

        drop(replacement);
        drop(second);
        assert_eq!(budget.available_global_bytes(), 10);
    }

    #[test]
    fn repeated_saturation_never_consumes_more_than_the_fixed_budget() {
        let budget = PacketQueueBudget::new(64, 32);
        let client = budget.client();

        for _ in 0..10_000 {
            let reservation = client.try_reserve(32).expect("full local budget");
            assert!(client.try_reserve(1).is_none());
            assert_eq!(client.available_local_bytes(), 0);
            assert_eq!(budget.available_global_bytes(), 32);
            drop(reservation);
            assert_eq!(client.available_local_bytes(), 32);
            assert_eq!(budget.available_global_bytes(), 64);
        }
    }

    #[test]
    fn protocol_sized_packet_fits_the_default_per_client_budget() {
        let budget = PacketQueueBudget::default();
        let client = budget.client();
        let reservation = client
            .try_reserve(crate::protos::relay::MAX_PACKET_SIZE)
            .expect("one maximum protocol packet must remain admissible");
        assert_eq!(
            client.available_local_bytes(),
            PER_CLIENT_OUTGOING_PACKET_BYTES - crate::protos::relay::MAX_PACKET_SIZE
        );
        drop(reservation);
    }
}
