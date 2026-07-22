use std::io;
use std::sync::Mutex;

use super::core::eio;
use super::wire::{FsResponse, MOUNT_PATH_CAPABILITY_CONTRACT_VERSION};

const MAX_MOUNT_LEASE_LENGTH: usize = 128;

#[derive(Default)]
pub(super) struct PeerMountLeaseClient {
    token: Mutex<Option<String>>,
}

impl PeerMountLeaseClient {
    pub(super) fn current(&self) -> io::Result<Option<String>> {
        self.token
            .lock()
            .map(|lease| lease.clone())
            .map_err(|_| eio("Peer-Mount-Lease ist gesperrt"))
    }

    pub(super) fn accept_capabilities(
        &self,
        response: FsResponse,
        acquire_lease: bool,
    ) -> io::Result<crate::vfs::MountPathCapabilities> {
        let FsResponse::Capabilities {
            capabilities,
            contract_version,
            root_confined,
            lease,
        } = response
        else {
            return Err(eio("unerwartete Antwort auf capabilities"));
        };
        if contract_version == 0 {
            if acquire_lease {
                self.replace(None)?;
            }
            return Ok(crate::vfs::MountPathCapabilities::default());
        }
        if contract_version != MOUNT_PATH_CAPABILITY_CONTRACT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "Peer-Mount-Capability-Vertrag v{contract_version} wird nicht unterstuetzt"
                ),
            ));
        }
        if !acquire_lease {
            if lease.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Peer hat fuer eine reine Mount-Pruefung unerwartet eine Lease erzeugt",
                ));
            }
            return Ok(crate::vfs::MountPathCapabilities {
                staged_write: capabilities.into(),
                root_confinement: if root_confined {
                    crate::vfs::RootConfinement::Enforced
                } else {
                    crate::vfs::RootConfinement::Unverified
                },
            });
        }
        match (root_confined, lease) {
            (root_confined, Some(lease))
                if !lease.is_empty() && lease.len() <= MAX_MOUNT_LEASE_LENGTH =>
            {
                self.replace(Some(lease))?;
                Ok(crate::vfs::MountPathCapabilities {
                    staged_write: capabilities.into(),
                    root_confinement: if root_confined {
                        crate::vfs::RootConfinement::Enforced
                    } else {
                        crate::vfs::RootConfinement::Unverified
                    },
                })
            }
            (false, None) => {
                self.replace(None)?;
                // A lease-free v1 response is the synthetic/read-only escape
                // hatch. Never trust advertised write bits without the token
                // that binds subsequent mutations to this exact root.
                Ok(crate::vfs::MountPathCapabilities::default())
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Peer-Mount-Capability-Antwort enthaelt eine inkonsistente Root-Lease",
            )),
        }
    }

    pub(super) fn clear(&self) -> io::Result<()> {
        self.replace(None)
    }

    fn replace(&self, lease: Option<String>) -> io::Result<()> {
        *self
            .token
            .lock()
            .map_err(|_| eio("Peer-Mount-Lease ist gesperrt"))? = lease;
        Ok(())
    }
}
