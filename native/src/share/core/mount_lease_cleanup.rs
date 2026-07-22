use std::sync::Arc;

use super::mount_lease::PeerMountLeases;

pub(super) struct LegacyLeaseCleanup {
    leases: Arc<PeerMountLeases>,
    connection: usize,
    runtime: Arc<tokio::runtime::Runtime>,
}

impl LegacyLeaseCleanup {
    pub(super) fn new(
        leases: Arc<PeerMountLeases>,
        connection: usize,
        runtime: Arc<tokio::runtime::Runtime>,
    ) -> Self {
        Self {
            leases,
            connection,
            runtime,
        }
    }
}

impl Drop for LegacyLeaseCleanup {
    fn drop(&mut self) {
        let Ok(leases) = self.leases.take_legacy_connection(self.connection) else {
            return;
        };
        if leases.is_empty() {
            return;
        }
        // A retained target may own an SFTP/Peer Tokio runtime. Never perform
        // its final Drop on the async Iroh connection task or under the table
        // lock; the existing bounded blocking pool owns disposal.
        self.runtime.spawn(async move {
            let _ = super::blocking::run("Share dispose legacy mount leases", move || {
                drop(leases);
                Ok(())
            })
            .await;
        });
    }
}
