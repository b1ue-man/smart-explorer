//! Capability queries and the conservative contract of admitted legacy peers.
//! Neither a transport failure nor an unsigned signaling answer proves an
//! old peer's admission. The ordinary pinned session and ListDir do that.
use std::io;
use std::time::{Duration, Instant};

use crate::vfs::{MountPathCapabilities, VfsMeta, VfsResult};

use super::backend::PeerBackend;
use super::core::eio;
use super::direct_ledger::{DirectRelayOutcome, DirectRequestDirection};
use super::direct_lifecycle::DirectDecisionState;
use super::io_deadline;
use super::profiles::fingerprint_matches;
use super::signal_presence::PresenceSignature;
use super::types::{DirectAccessState, PeerEndpoint, PeerOpenTarget, ShareAuthState, ShareScope};
use super::wire::{FsRequest, FsResponse};

const MOUNT_CAPABILITY_PROBE_TIMEOUT: Duration = Duration::from_secs(40);

impl PeerBackend {
    /// Used only after the explicit legacy proof has checked its binding.
    /// Do not discover optional features before the old peer's ListDir.
    pub(super) fn probe_legacy_root(&self) -> io::Result<Vec<VfsMeta>> {
        self.probe_legacy_root_until(Instant::now() + MOUNT_CAPABILITY_PROBE_TIMEOUT)
    }

    fn probe_legacy_root_until(&self, deadline: Instant) -> io::Result<Vec<VfsMeta>> {
        match self.request_unleased_until(FsRequest::ListDir { path: "/".into() }, deadline)? {
            FsResponse::Entries { entries } => Ok(entries.into_iter().map(Into::into).collect()),
            _ => Err(eio("unerwartete Antwort auf die Legacy-Rootprobe")),
        }
    }

    pub(super) fn legacy_capabilities(&self) -> io::Result<bool> {
        self.legacy_capabilities_until(Instant::now() + MOUNT_CAPABILITY_PROBE_TIMEOUT)
    }

    fn legacy_capabilities_until(&self, deadline: Instant) -> io::Result<bool> {
        io_deadline::remaining(deadline, "peer capability binding")?;
        self.node.require_sharing_active()?;
        let endpoint = self.current_endpoint()?;
        if !self.endpoint_source.legacy_direct(&endpoint)? {
            return Ok(false);
        }
        // On a fresh/restarted service, the stored contact alone must not be
        // mistaken for current peer admission. No lock crosses this probe.
        if self.node.outgoing_generation(&endpoint).is_none() {
            self.probe_legacy_root_until(deadline)?;
        }
        self.node.require_sharing_active()?;
        let endpoint = self.current_endpoint()?;
        self.endpoint_source.legacy_direct(&endpoint)
    }

    pub(crate) fn probe_mount_path_capabilities(
        &self,
        root: &str,
    ) -> VfsResult<MountPathCapabilities> {
        self.probe_mount_path_capabilities_until(
            root,
            Instant::now() + MOUNT_CAPABILITY_PROBE_TIMEOUT,
        )
    }

    pub(crate) fn probe_mount_path_capabilities_until(
        &self,
        root: &str,
        deadline: Instant,
    ) -> VfsResult<MountPathCapabilities> {
        self.query_mount_path_capabilities(root, false, deadline)
    }

    pub(super) fn query_mount_path_capabilities(
        &self,
        root: &str,
        acquire_lease: bool,
        deadline: Instant,
    ) -> VfsResult<MountPathCapabilities> {
        if acquire_lease {
            // Preserve release before every failed replacement/probe too.
            self.release_current_mount_lease();
        }
        if self.legacy_capabilities_until(deadline)? {
            if acquire_lease {
                self.mount_lease.clear()?;
            }
            // No old protocol operation establishes root confinement or
            // safe mounted writes. Ordinary authorized requests remain
            // stateless; this branch never creates a mount-root lease.
            return Ok(MountPathCapabilities::default());
        }
        let response = self.request_unleased_until(
            FsRequest::Capabilities {
                path: root.to_string(),
                acquire_lease,
                lease_request_id: acquire_lease
                    .then(|| super::core::random_token(16).map_err(eio))
                    .transpose()?,
            },
            deadline,
        )?;
        self.mount_lease.accept_capabilities(response, acquire_lease)
    }
}

/// This is protocol classification, never a grant or an accepted decision.
/// A matching current outgoing relation and real pinned admission are both
/// required; an EOF, unknown-variant message or server advert is not evidence.
pub(super) fn is_legacy_direct(
    state: &ShareAuthState,
    target: &PeerOpenTarget,
    endpoint: &PeerEndpoint,
) -> io::Result<bool> {
    let PeerOpenTarget::Direct { contact_id } = target else {
        return Ok(false);
    };
    let ShareScope::Direct {
        contact_id: scoped,
    } = &endpoint.scope else {
        return Ok(false);
    };
    let Some(contact) = state
        .direct_contacts
        .iter()
        .find(|contact| &contact.id == contact_id)
    else {
        return Ok(false);
    };
    let peer = &endpoint.presence;
    if peer.signature_state() != PresenceSignature::Unsigned {
        return Ok(false);
    }
    if contact.relation.signed_presence || super::signal_auth::peer_signature_seen(state, peer) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Bekannt signiertes Direktgeraet darf nicht auf Legacy zurueckstufen",
        ));
    }
    if !state.direct_online {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Direktfreigabe ist offline",
        ));
    }
    if scoped != contact_id
        || contact.access_state != DirectAccessState::Accepted
        || contact.accepted_at.is_none()
        || contact.accepted_public_key.as_deref() != Some(peer.public_key.as_str())
        || contact.expected_node_id != peer.node_id
        || endpoint.expected_node_id.as_deref() != Some(peer.node_id.as_str())
        || peer.node_id != peer.public_key
        || contact.expected_fingerprint != peer.fingerprint
        || !fingerprint_matches(&peer.public_key, &contact.expected_fingerprint)
        || contact.remote_device_id.as_deref() != Some(peer.device_id.as_str())
        || contact.remote_public_key.as_deref() != Some(peer.public_key.as_str())
        || peer.kind != "direct"
        || peer.relation_id != contact.lookup_id
    {
        return Ok(false);
    }
    let mut entries = state.direct_requests.iter().filter(|entry| {
        entry.direction == DirectRequestDirection::Outgoing
            && entry.contact_id.as_deref() == Some(contact_id.as_str())
    });
    let Some(entry) = entries.next() else {
        return Ok(false);
    };
    let request = &entry.record.request;
    Ok(entries.next().is_none()
        && entry.request_receipt.is_none()
        && entry.decision.is_none()
        && entry.decision_receipt.is_none()
        && entry.record.decision.state == DirectDecisionState::Pending
        && entry.record.decision.revision == 0
        && entry.retries.request.relay_outcome == Some(DirectRelayOutcome::LegacyForwarded)
        && !entry
            .local_lookup_id
            .as_deref()
            .is_some_and(|lookup| lookup != state.identity.direct_lookup_id.as_str())
        && request.lookup_id == contact.lookup_id
        && request.requester.device_id == state.identity.device_id
        && request.requester.public_key == state.identity.public_key
        && request.requester.node_id == state.identity.node_id
        && request.requester.fingerprint == state.identity.fingerprint
        && request.target.public_key == peer.public_key
        && request.target.node_id == peer.node_id
        && request.target.fingerprint == peer.fingerprint
        && (request.target.device_id.is_empty() || request.target.device_id == peer.device_id))
}
