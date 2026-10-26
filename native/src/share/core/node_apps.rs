//! Catalog-bound app authority and durable peer ownership; no Exec fallback.
use super::{
    access_approval::{ApprovalOutcome, ApprovalSummary},
    access_authorize::{capability_for, principal_grant, request_for},
    access_constraints::{self as constraints, AccessRequest},
    access_grant::{Capability, CapabilityDecision},
    access_revocation::{AccessConfirmation, AccessHandle},
    apps_wire::{self, AppsClientHello, AppsCommand, AppsReply, AppsResult, AppsServerHello},
    core::{eio, now_secs, random_token},
    handshake_limits::ApplicationHandshakePermit,
    node::ShareIrohNode,
    relation_proof::{prove_relation, ProvenPrincipal},
    session::PeerPrincipal,
};
use crate::apps::{
    AppHost, CatalogIcons, CatalogSnapshot, DeviceId, LaunchAdmission, LaunchDenial, LaunchFacts,
    LaunchReceipt, LaunchRequest,
};
use iroh::endpoint::Connection;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
    time::Duration,
};
use tokio::sync::Semaphore;
#[path = "node_apps_client.rs"]
mod client;
#[path = "node_apps_host.rs"]
mod host;
#[path = "node_apps_platform.rs"]
mod platform;
use host::access_request;
use platform::install_platform;

#[path = "node_apps_window.rs"]
mod window;

struct Configuration {
    host: Arc<AppHost>,
}
#[derive(Clone)]
struct PendingApp {
    principal: ProvenPrincipal,
    request: LaunchRequest,
    access: Option<Arc<AccessHandle>>,
}
pub(crate) struct NodeApps {
    configuration: Mutex<Option<Configuration>>,
    pending: Mutex<BTreeMap<String, PendingApp>>,
    window_watches: Mutex<std::collections::BTreeSet<String>>,
    pub(super) slots: Arc<Semaphore>,
}
impl NodeApps {
    pub(super) fn new() -> Self {
        Self {
            configuration: Mutex::new(None),
            pending: Mutex::new(BTreeMap::new()),
            window_watches: Mutex::new(Default::default()),
            slots: Arc::new(Semaphore::new(16)),
        }
    }
    fn host(&self, principal: &ProvenPrincipal) -> io::Result<Arc<AppHost>> {
        let _ = principal;
        self.local_host()
    }
    fn local_host(&self) -> io::Result<Arc<AppHost>> {
        let configuration = self
            .configuration
            .lock()
            .map_err(|_| eio("app host configuration unavailable"))?;
        let config = configuration.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "interactive app host is not installed",
            )
        })?;
        Ok(config.host.clone())
    }
}
fn owner_key(principal: &ProvenPrincipal) -> io::Result<String> {
    let bytes = serde_json::to_vec(&(
        &principal.relation_kind,
        &principal.relation_id,
        &principal.device_id,
        &principal.public_key,
        &principal.fingerprint,
        &principal.node_id,
    ))
    .map_err(io::Error::other)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
pub(super) fn stop_all(apps: &NodeApps) {
    if let Ok(mut pending) = apps.pending.lock() {
        pending.clear();
    }
}
impl super::service::ShareService {
    /// Local daemon-token callers only. No network command exposes this inventory.
    pub(crate) fn local_app_grants(
        &self,
        device: &str,
        key: &str,
    ) -> io::Result<crate::apps::LocalAppGrantSnapshot> {
        let catalog = self.iroh.apps.local_host()?.catalog()?;
        let own = crate::share::exec_provider_status().user_label;
        let rights = self
            .device_access_view(device, key, &own, now_secs())
            .map_err(eio)?;
        Ok(crate::apps::LocalAppGrantSnapshot {
            peer_device_id: device.into(),
            peer_public_key: key.into(),
            catalog,
            relations: rights.relations,
        })
    }
    /// Build a minimal AppLaunch patch from current host facts/current exact
    /// source. Persistence repeats the supplied revision; every unrelated field
    /// survives because account/programs/global expiry are omitted and the
    /// other V2 conditions are copied at that same revision.
    pub(crate) fn prepare_local_app_grant(
        &self,
        change: &crate::apps::LocalAppGrantChange,
    ) -> io::Result<super::access_grant::AccessGrantChange> {
        use super::{access_constraints::AppLaunchConstraints, access_grant::AccessGrantChange};
        let (device, key) = match &change.target {
            super::types::ExecGrantTarget::Direct {
                device_id,
                public_key,
                ..
            }
            | super::types::ExecGrantTarget::RoomMember {
                device_id,
                public_key,
                ..
            } => (device_id.as_str(), public_key.as_str()),
        };
        if change.host_device_id.as_str() != self.identity.device_id {
            return Err(eio("local_host_identity_changed; reload app grant editor"));
        }
        let own = crate::share::exec_provider_status().user_label;
        let rights = self
            .device_access_view(device, key, &own, now_secs())
            .map_err(eio)?;
        let source = rights
            .relations
            .into_iter()
            .find(|r| r.target == change.target)
            .ok_or_else(|| eio("exact app grant relationship no longer exists"))?;
        if source.policy_revision != change.expected_policy_revision {
            return Err(eio(
                "app grant policy revision changed; reload without overwriting",
            ));
        }
        let mut patch = AccessGrantChange {
            capabilities: BTreeMap::from([(Capability::AppLaunch, change.decision)]),
            ..Default::default()
        };
        // Revoking never needs a working GUI/catalog and leaves the previous
        // bounds intact. Re-enabling always needs a fresh explicit binding.
        if change.decision == CapabilityDecision::Off {
            return Ok(patch);
        }
        if !source.active {
            return Err(eio("app grant relationship is inactive"));
        }
        let catalog = self.iroh.apps.local_host()?.catalog()?;
        if catalog.device_id != change.host_device_id
            || catalog.revision != change.catalog_revision
            || catalog.session.user_id != change.user_id
            || catalog.authorization_scope != change.authorization_scope
        {
            return Err(eio(
                "local_app_catalog_or_login_changed; explicit rebinding required",
            ));
        }
        if !catalog.session.interactive {
            return Err(eio("local host has no interactive app session"));
        }
        if change.app_ids.is_empty()
            || change.app_ids.len() > catalog.apps.len()
            || change
                .app_ids
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != change.app_ids.len()
            || change.app_ids.iter().any(|id| {
                !catalog
                    .apps
                    .iter()
                    .any(|app| app.app_id == *id && app.can_launch)
            })
        {
            return Err(eio(
                "select explicit available IDs from the current local app catalog",
            ));
        }
        if change.valid_until.is_some_and(|end| end <= now_secs()) {
            return Err(eio("app grant expiry must be in the future"));
        }
        let mut constraints = source.grant.constraints;
        constraints.app_launch = Some(AppLaunchConstraints {
            app_ids: change
                .app_ids
                .iter()
                .map(|id| constraints::AppId::parse(id.as_str()).map_err(eio))
                .collect::<io::Result<_>>()?,
            sessions: vec![
                constraints::SessionId::parse(catalog.authorization_scope.as_str()).map_err(eio)?,
            ],
            users: vec![constraints::UserId::parse(catalog.session.user_id.as_str()).map_err(eio)?],
            valid_until: change.valid_until,
        });
        constraints.validate().map_err(eio)?;
        patch.constraints = Some(constraints);
        Ok(patch)
    }
    pub(crate) fn configure_apps_host(
        &self,
        device: DeviceId,
        root: PathBuf,
        icons: Option<CatalogIcons>,
    ) -> io::Result<()> {
        if device.as_str() != self.identity.device_id {
            return Err(eio("app host device identity mismatch"));
        }
        let mut configuration = self
            .iroh
            .apps
            .configuration
            .lock()
            .map_err(|_| eio("app host configuration unavailable"))?;
        if configuration.is_some() {
            return Ok(());
        }
        let host = AppHost::for_current_user(device, &root, icons)?;
        install_platform(Arc::downgrade(&self.iroh))?;
        *configuration = Some(Configuration {
            host: Arc::new(host),
        });
        Ok(())
    }
}
