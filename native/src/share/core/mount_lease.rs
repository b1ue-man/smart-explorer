use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use super::core::{eio, random_token};
use super::fs::{self, ResolvedTarget, ShareExportConfig};
use super::fs_capabilities::ResolvedMountCapabilities;
use super::session::IncomingSession;

const MAX_MOUNT_LEASES_PER_CONNECTION: usize = 16;
const MOUNT_LEASE_RANDOM_BYTES: usize = 32;
const MAX_WIRE_LEASE_LENGTH: usize = 128;

pub(super) struct PeerMountLease {
    target: ResolvedTarget,
    capabilities: crate::vfs::MountPathCapabilities,
    virtual_root: String,
    virtual_components: Vec<String>,
    exports: ShareExportConfig,
    authorization_epoch: u64,
    backend_identity: String,
}

impl PeerMountLease {
    fn new(
        resolved: ResolvedMountCapabilities,
        exports: ShareExportConfig,
        authorization_epoch: u64,
    ) -> io::Result<Self> {
        let virtual_components = fs::split_clean(&resolved.virtual_root)?;
        if virtual_components.is_empty() {
            return Err(eio(
                "Synthetische Share-Wurzel kann keine Mount-Lease erhalten",
            ));
        }
        let mut capabilities = resolved.capabilities;
        capabilities.root_confinement = if resolved.lease_root_confined() {
            crate::vfs::RootConfinement::Enforced
        } else {
            crate::vfs::RootConfinement::Unverified
        };
        let backend_identity = resolved.target.backend.state_identity();
        Ok(Self {
            target: resolved.target,
            capabilities,
            virtual_root: resolved.virtual_root,
            virtual_components,
            exports,
            authorization_epoch,
            backend_identity,
        })
    }

    pub(super) fn capabilities(&self) -> crate::vfs::MountPathCapabilities {
        self.capabilities
    }

    pub(super) fn authorize(
        &self,
        current: &ShareExportConfig,
        authorization_epoch: u64,
    ) -> io::Result<()> {
        if self.authorization_epoch != authorization_epoch {
            Err(permission_denied(
                "Freigabe-Autorisierung wurde seit dem Einbinden erneuert; Laufwerk muss neu verbunden werden",
            ))
        } else if &self.exports == current {
            Ok(())
        } else {
            Err(permission_denied(
                "Freigaben wurden seit dem Einbinden geaendert; Laufwerk muss neu verbunden werden",
            ))
        }
    }

    fn same_binding(&self, other: &Self) -> bool {
        self.virtual_root == other.virtual_root
            && self.exports == other.exports
            && self.authorization_epoch == other.authorization_epoch
            && self.backend_identity == other.backend_identity
            && self.target.mount_key == other.target.mount_key
            && self.target.path == other.target.path
    }

    pub(super) fn resolve(&self, virtual_path: &str) -> io::Result<ResolvedTarget> {
        let components = fs::split_clean(virtual_path)?;
        let relative = components
            .strip_prefix(self.virtual_components.as_slice())
            .ok_or_else(|| {
                permission_denied(format!(
                    "Pfad liegt ausserhalb der eingebundenen Peer-Wurzel {}",
                    self.virtual_root
                ))
            })?;
        let mut target = self.target.clone();
        target.path = if target.backend.is_local() {
            // Repeat canonical ancestor validation for every Local/UNC path as
            // defense in depth. This does not claim race-proof confinement;
            // those backends remain Unverified and require trusted-root mode.
            fs::secure_local_target(&self.target.path, relative)?
        } else {
            fs::join_under(&self.target.path, relative)
        };
        Ok(target)
    }
}

pub(super) struct MountLeaseGrant {
    pub(super) token: String,
    pub(super) lease: Arc<PeerMountLease>,
}

#[derive(Default)]
pub(super) struct PeerMountLeases {
    entries: Mutex<HashMap<String, Arc<PeerMountLease>>>,
}

impl PeerMountLeases {
    pub(super) fn acquire(
        &self,
        resolved: ResolvedMountCapabilities,
        exports: ShareExportConfig,
        authorization_epoch: u64,
    ) -> io::Result<MountLeaseGrant> {
        let candidate = PeerMountLease::new(resolved, exports, authorization_epoch)?;
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| eio("Peer-Mount-Lease-Tabelle ist gesperrt"))?;
        if let Some((token, lease)) = entries
            .iter()
            .find(|(_, lease)| lease.same_binding(&candidate))
        {
            return Ok(MountLeaseGrant {
                token: token.clone(),
                lease: lease.clone(),
            });
        }
        if entries.len() >= MAX_MOUNT_LEASES_PER_CONNECTION {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Zu viele unterschiedliche Peer-Laufwerkswurzeln in derselben Verbindung; Share-Verbindung neu aufbauen",
            ));
        }
        let lease = Arc::new(candidate);
        for _ in 0..4 {
            let token = random_token(MOUNT_LEASE_RANDOM_BYTES).map_err(eio)?;
            if !entries.contains_key(&token) {
                entries.insert(token.clone(), lease.clone());
                return Ok(MountLeaseGrant { token, lease });
            }
        }
        Err(eio(
            "Eindeutige Peer-Mount-Lease konnte nicht erzeugt werden",
        ))
    }

    pub(super) fn authorize(
        &self,
        token: &str,
        exports: &ShareExportConfig,
        authorization_epoch: u64,
    ) -> io::Result<Arc<PeerMountLease>> {
        if token.is_empty() || token.len() > MAX_WIRE_LEASE_LENGTH {
            return Err(permission_denied("Ungueltige Peer-Mount-Lease"));
        }
        let lease = self
            .entries
            .lock()
            .map_err(|_| eio("Peer-Mount-Lease-Tabelle ist gesperrt"))?
            .get(token)
            .cloned()
            .ok_or_else(|| {
                permission_denied(
                    "Peer-Mount-Lease gehoert nicht zu dieser Verbindung; Laufwerk neu verbinden",
                )
            })?;
        lease.authorize(exports, authorization_epoch)?;
        Ok(lease)
    }
}

#[derive(Clone)]
pub(super) struct MountLeaseAuthorization {
    token: String,
    lease: Arc<PeerMountLease>,
    session: Arc<IncomingSession>,
    auth: Arc<Mutex<super::types::ShareAuthState>>,
    node: Arc<super::node::ShareIrohNode>,
}

impl MountLeaseAuthorization {
    pub(super) fn new(
        token: String,
        lease: Arc<PeerMountLease>,
        session: Arc<IncomingSession>,
        auth: Arc<Mutex<super::types::ShareAuthState>>,
        node: Arc<super::node::ShareIrohNode>,
    ) -> Self {
        Self {
            token,
            lease,
            session,
            auth,
            node,
        }
    }

    pub(super) fn verify_token(&self, echoed_token: Option<&str>) -> io::Result<()> {
        if echoed_token != Some(self.token.as_str()) {
            return Err(permission_denied(
                "Peer-Mount-Lease fehlt beim Schreibabschluss",
            ));
        }
        Ok(())
    }

    pub(super) fn run<T>(&self, operation: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        // The auth mutex defines the admission order against revoke/config
        // updates, but is released before potentially slow filesystem/network
        // I/O. If revoke wins the lock first this fails closed; if this check
        // wins, the already-admitted mutation may complete without delaying
        // the later revoke for the duration of remote I/O.
        self.node.require_sharing_active()?;
        let current = self.session.authorize(&self.auth)?;
        let authorization_epoch = self.node.filesystem_authorization_epoch();
        self.lease.authorize(&current, authorization_epoch)?;
        operation()
    }
}

pub(super) fn run_authorized<T>(
    authorization: Option<&MountLeaseAuthorization>,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    match authorization {
        Some(authorization) => authorization.run(operation),
        None => operation(),
    }
}

fn permission_denied(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message.into())
}
