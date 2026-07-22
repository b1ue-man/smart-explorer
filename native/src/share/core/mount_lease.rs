use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use super::core::{eio, random_token};
use super::fs::{self, ResolvedTarget, ShareExportConfig};
use super::fs_capabilities::ResolvedMountCapabilities;
use super::session::{IncomingSession, PeerPrincipal};

const MAX_MOUNT_LEASES_PER_PRINCIPAL: usize = 4;
const MAX_MOUNT_LEASES_TOTAL: usize = 16;
const MOUNT_LEASE_RANDOM_BYTES: usize = 32;
const MAX_WIRE_LEASE_LENGTH: usize = 128;
const MAX_LEASE_REQUEST_ID_LENGTH: usize = 128;
pub(super) const RELEASABLE_LEASE_PREFIX: &str = "se-mount-v2.";

pub(super) struct PeerMountLease {
    target: ResolvedTarget,
    capabilities: crate::vfs::MountPathCapabilities,
    virtual_root: String,
    virtual_components: Vec<String>,
    exports: ShareExportConfig,
    principal: PeerPrincipal,
    lease_request_id: Option<String>,
    legacy_connection: usize,
    authorization_epoch: u64,
    backend_identity: String,
}

impl PeerMountLease {
    fn new(
        resolved: ResolvedMountCapabilities,
        exports: ShareExportConfig,
        principal: PeerPrincipal,
        lease_request_id: Option<String>,
        legacy_connection: usize,
        authorization_epoch: u64,
    ) -> io::Result<Self> {
        if lease_request_id
            .as_deref()
            .is_some_and(|id| id.is_empty() || id.len() > MAX_LEASE_REQUEST_ID_LENGTH)
        {
            return Err(permission_denied("Ungueltige Mount-Anforderungs-ID"));
        }
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
            principal,
            lease_request_id,
            legacy_connection,
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
        principal: &PeerPrincipal,
        legacy_connection: usize,
        authorization_epoch: u64,
    ) -> io::Result<()> {
        if &self.principal != principal {
            Err(permission_denied(
                "Peer-Mount-Lease gehoert nicht zu dieser authentifizierten Identitaet",
            ))
        } else if self.lease_request_id.is_none() && self.legacy_connection != legacy_connection {
            Err(permission_denied(
                "Legacy-Mount-Lease gehoert zu einer beendeten Verbindung",
            ))
        } else if self.authorization_epoch != authorization_epoch {
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
            && self.principal == other.principal
            && (match (&self.lease_request_id, &other.lease_request_id) {
                (Some(left), Some(right)) => left == right,
                (None, None) => self.legacy_connection == other.legacy_connection,
                _ => false,
            })
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
    pub(super) fn existing_acquisition(
        &self,
        virtual_root: &str,
        exports: &ShareExportConfig,
        principal: &PeerPrincipal,
        lease_request_id: Option<&str>,
        legacy_connection: usize,
        authorization_epoch: u64,
    ) -> io::Result<Option<MountLeaseGrant>> {
        validate_request_id(lease_request_id)?;
        let entries = self
            .entries
            .lock()
            .map_err(|_| eio("Peer-Mount-Lease-Tabelle ist gesperrt"))?;
        let existing = entries.iter().find(|(_, lease)| {
            &lease.principal == principal
                && (match (lease.lease_request_id.as_deref(), lease_request_id) {
                    (Some(left), Some(right)) => left == right,
                    (None, None) => lease.legacy_connection == legacy_connection,
                    _ => false,
                })
        });
        let Some((token, lease)) = existing else {
            return Ok(None);
        };
        if lease.virtual_root != virtual_root
            || &lease.exports != exports
            || lease.authorization_epoch != authorization_epoch
        {
            return Err(permission_denied(
                "Mount-Anforderungs-ID wurde fuer eine andere Root- oder Policy-Bindung wiederverwendet",
            ));
        }
        Ok(Some(MountLeaseGrant {
            token: token.clone(),
            lease: lease.clone(),
        }))
    }

    pub(super) fn acquire(
        &self,
        resolved: ResolvedMountCapabilities,
        exports: ShareExportConfig,
        principal: PeerPrincipal,
        lease_request_id: Option<String>,
        legacy_connection: usize,
        authorization_epoch: u64,
    ) -> io::Result<MountLeaseGrant> {
        let candidate = PeerMountLease::new(
            resolved,
            exports,
            principal.clone(),
            lease_request_id,
            legacy_connection,
            authorization_epoch,
        )?;
        let stale = {
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| eio("Peer-Mount-Lease-Tabelle ist gesperrt"))?;
            let tokens: Vec<String> = entries
                .iter()
                .filter(|(_, lease)| {
                    lease.principal == candidate.principal
                        && (lease.authorization_epoch != candidate.authorization_epoch
                            || lease.exports != candidate.exports)
                })
                .map(|(token, _)| token.clone())
                .collect();
            tokens
                .into_iter()
                .filter_map(|token| entries.remove(&token))
                .collect::<Vec<_>>()
        };
        // Runtime-owning targets are disposed only after the table lock is
        // released. Acquire itself already runs on the bounded blocking path.
        drop(stale);
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
        if entries.len() >= MAX_MOUNT_LEASES_TOTAL {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Globale Grenze fuer aktive Peer-Laufwerkswurzeln erreicht",
            ));
        }
        if entries
            .values()
            .filter(|lease| lease.principal == principal)
            .count()
            >= MAX_MOUNT_LEASES_PER_PRINCIPAL
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Zu viele aktive Laufwerkswurzeln fuer diese Peer-Identitaet",
            ));
        }
        let lease = Arc::new(candidate);
        for _ in 0..4 {
            let token = format!(
                "{}{}",
                RELEASABLE_LEASE_PREFIX,
                random_token(MOUNT_LEASE_RANDOM_BYTES).map_err(eio)?
            );
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
        principal: &PeerPrincipal,
        exports: &ShareExportConfig,
        legacy_connection: usize,
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
                permission_denied("Peer-Mount-Lease ist unbekannt; Laufwerk neu verbinden")
            })?;
        lease.authorize(exports, principal, legacy_connection, authorization_epoch)?;
        Ok(lease)
    }

    pub(super) fn release(
        &self,
        token: &str,
        principal: &PeerPrincipal,
    ) -> io::Result<Option<Arc<PeerMountLease>>> {
        if token.is_empty() || token.len() > MAX_WIRE_LEASE_LENGTH {
            return Err(permission_denied("Ungueltige Peer-Mount-Lease"));
        }
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| eio("Peer-Mount-Lease-Tabelle ist gesperrt"))?;
        match entries.get(token).map(|lease| lease.principal.clone()) {
            Some(owner) if &owner == principal => Ok(entries.remove(token)),
            Some(_) => Err(permission_denied(
                "Peer-Mount-Lease gehoert nicht zu dieser authentifizierten Identitaet",
            )),
            None => Ok(None),
        }
    }

    pub(super) fn take_legacy_connection(
        &self,
        connection: usize,
    ) -> io::Result<Vec<Arc<PeerMountLease>>> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| eio("Peer-Mount-Lease-Tabelle ist gesperrt"))?;
        let tokens: Vec<String> = entries
            .iter()
            .filter(|(_, lease)| {
                lease.lease_request_id.is_none() && lease.legacy_connection == connection
            })
            .map(|(token, _)| token.clone())
            .collect();
        Ok(tokens
            .into_iter()
            .filter_map(|token| entries.remove(&token))
            .collect())
    }

    pub(super) fn clear(&self) -> io::Result<()> {
        let removed: Vec<_> = self
            .entries
            .lock()
            .map_err(|_| eio("Peer-Mount-Lease-Tabelle ist gesperrt"))?
            .drain()
            .map(|(_, lease)| lease)
            .collect();
        drop(removed);
        Ok(())
    }
}

pub(super) struct MountLeaseAuthorization {
    token: String,
    lease: Arc<PeerMountLease>,
    session: Arc<IncomingSession>,
    auth: Arc<Mutex<super::types::ShareAuthState>>,
    node: Arc<super::node::ShareIrohNode>,
    legacy_connection: usize,
}

impl MountLeaseAuthorization {
    pub(super) fn new(
        token: String,
        lease: Arc<PeerMountLease>,
        session: Arc<IncomingSession>,
        auth: Arc<Mutex<super::types::ShareAuthState>>,
        node: Arc<super::node::ShareIrohNode>,
        legacy_connection: usize,
    ) -> Self {
        Self {
            token,
            lease,
            session,
            auth,
            node,
            legacy_connection,
        }
    }

    pub(super) fn token(&self) -> &str {
        &self.token
    }

    pub(super) fn run<T>(&self, operation: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        // The auth mutex defines the admission order against revoke/config
        // updates, but is released before potentially slow filesystem/network
        // I/O. If revoke wins the lock first this fails closed; if this check
        // wins, the already-admitted mutation may complete without delaying
        // the later revoke for the duration of remote I/O.
        self.node.require_sharing_active()?;
        let current = self.session.authorize(&self.auth)?;
        let principal = self.session.principal();
        let authorization_epoch = self.node.filesystem_authorization_epoch();
        self.lease.authorize(
            &current,
            &principal,
            self.legacy_connection,
            authorization_epoch,
        )?;
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

fn validate_request_id(id: Option<&str>) -> io::Result<()> {
    if id.is_some_and(|id| id.is_empty() || id.len() > MAX_LEASE_REQUEST_ID_LENGTH) {
        Err(permission_denied("Ungueltige Mount-Anforderungs-ID"))
    } else {
        Ok(())
    }
}
