//! Per-process view of a running Direct-code renewal (E17), keyed by the
//! local Iroh node id.
//!
//! `ShareIdentity` and `ShareAuthState` stay unchanged (their literals are
//! part of many tests), so the retiring data live here. The durable store
//! (`os/shared/code_renewal_store.rs`) installs a node's view on every
//! production identity load and after every change of the renewal file.
//! Without a view every check behaves exactly as before the renewal existed.
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard, OnceLock};

use zeroize::Zeroizing;

use super::code_renewal::CodeRenewalStatus;

/// Removed identities (node ids and keys) remembered for early refusal.
pub const MAX_WITHDRAWN_KEYS: usize = 4096;

/// One retiring lookup id with its secret and the devices still on it.
pub struct RetiringMaterial {
    pub lookup_id: String,
    pub secret: Zeroizing<Vec<u8>>,
    /// `(device_id, public_key, node_id)` of every pending device.
    pub pending: Vec<(String, String, String)>,
}

#[derive(Default)]
struct NodeView {
    current_lookup_id: String,
    retiring: Vec<RetiringMaterial>,
    withdrawn: HashSet<String>,
    published: HashSet<String>,
    status: CodeRenewalStatus,
}

fn views() -> MutexGuard<'static, HashMap<String, NodeView>> {
    static VIEWS: OnceLock<Mutex<HashMap<String, NodeView>>> = OnceLock::new();
    // Every update replaces a node's view as a whole, so a poisoned lock
    // still holds a complete view.
    VIEWS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Replaces the view of `node_id`. The published set survives, so retired
/// lookup ids are still withdrawn from the server.
pub fn install(
    node_id: &str,
    current_lookup_id: &str,
    retiring: Vec<RetiringMaterial>,
    withdrawn: Vec<String>,
    status: CodeRenewalStatus,
) {
    if node_id.is_empty() {
        return;
    }
    let mut views = views();
    let view = views.entry(node_id.to_string()).or_default();
    view.current_lookup_id = current_lookup_id.to_string();
    view.retiring = retiring;
    view.withdrawn = withdrawn
        .into_iter()
        .filter(|key| !key.is_empty())
        .take(MAX_WITHDRAWN_KEYS)
        .collect();
    view.status = status;
}

/// Records a bookkeeping failure for the status of every surface.
pub fn record_error(node_id: &str, error: String) {
    let mut views = views();
    let view = views.entry(node_id.to_string()).or_default();
    view.status.error = Some(error);
}

/// The secret of `lookup_id` while it runs out for `node_id`.
pub fn retiring_secret(node_id: &str, lookup_id: &str) -> Option<Zeroizing<Vec<u8>>> {
    let views = views();
    let view = views.get(node_id)?;
    if lookup_id.is_empty() || lookup_id == view.current_lookup_id {
        return None;
    }
    view.retiring
        .iter()
        .find(|material| material.lookup_id == lookup_id)
        .map(|material| material.secret.clone())
}

/// Every retiring lookup id of `node_id` with its secret.
pub fn retiring_lookups(node_id: &str) -> Vec<(String, Zeroizing<Vec<u8>>)> {
    views()
        .get(node_id)
        .map(|view| {
            view.retiring
                .iter()
                .map(|material| (material.lookup_id.clone(), material.secret.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `lookup_id` runs out for any local identity of this process.
pub fn is_retiring_lookup(lookup_id: &str) -> bool {
    !lookup_id.is_empty()
        && views().values().any(|view| {
            view.retiring
                .iter()
                .any(|material| material.lookup_id == lookup_id)
        })
}

/// The retiring data the device `(device_id, public_key, node)` still
/// holds, so a push to it carries nothing new (`direct_reciprocal_renewal`).
pub fn pending_material_for(
    node_id: &str,
    device_id: &str,
    public_key: &str,
    peer_node: &str,
) -> Option<(String, Zeroizing<Vec<u8>>)> {
    let views = views();
    views.get(node_id)?.retiring.iter().find_map(|material| {
        material
            .pending
            .iter()
            .any(|(device, key, node)| {
                device == device_id
                    && key == public_key
                    && (node == peer_node || (node.is_empty() && key == peer_node))
            })
            .then(|| (material.lookup_id.clone(), material.secret.clone()))
    })
}

/// Whether a renewal of `node_id` waits for this device.
pub fn is_pending(node_id: &str, device_id: &str, public_key: &str) -> bool {
    views().get(node_id).is_some_and(|view| {
        view.retiring.iter().any(|material| {
            material
                .pending
                .iter()
                .any(|(device, key, _)| device == device_id && key == public_key)
        })
    })
}

/// Whether `endpoint` (node id or key) belongs to a device the user removed.
pub fn is_withdrawn(endpoint: &str) -> bool {
    !endpoint.is_empty()
        && views()
            .values()
            .any(|view| view.withdrawn.contains(endpoint))
}

/// Records what one publication round announced and returns the retiring
/// lookup ids announced before that are gone now (to unpublish).
pub fn sync_published(node_id: &str, published: &[String]) -> Vec<String> {
    let mut views = views();
    let Some(view) = views.get_mut(node_id) else {
        return Vec::new();
    };
    let now: HashSet<String> = published.iter().cloned().collect();
    let gone = view.published.difference(&now).cloned().collect::<Vec<_>>();
    view.published = now;
    gone
}

pub fn status(node_id: &str) -> CodeRenewalStatus {
    views()
        .get(node_id)
        .map(|view| view.status.clone())
        .unwrap_or_default()
}
