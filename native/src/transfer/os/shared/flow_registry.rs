//! The flows of this process by key: one flow per connection or local volume,
//! created on first use and forgotten when unused for a while, plus the
//! classification of errors that decides what an ended operation tells its
//! flow.
use super::flow::Flow;
use super::flow_control::{OpOutcome, RESOURCE_CEILING};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Flows unused for this long are forgotten (their connection is gone or idle).
const FLOW_RETENTION: Duration = Duration::from_secs(15 * 60);
/// A Share peer's flow starts at this many operations (K-E, 2026-10-28):
/// its streams share one QUIC connection without a handshake each and the
/// host admits 60 per device, so slow start need not climb from 2 through
/// several windows of round trips first.
const PEER_START_LIMIT: usize = 8;

struct Registry {
    flows: HashMap<String, (Arc<Flow>, Instant)>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            flows: HashMap::new(),
        })
    })
}

/// The flow for `key`, created on first use. `ceiling` is the backend's hard
/// protocol bound (`None` = only the resource guard applies).
pub fn flow(key: String, ceiling: Option<usize>) -> Arc<Flow> {
    flow_with_identity(key, None, ceiling)
}

/// Like `flow`; a new flow of a known `identity` (`Backend::flow_identity`:
/// the same across restarts, unlike the key) starts at half the limit it
/// learned last time.
pub fn flow_with_identity(
    key: String,
    identity: Option<String>,
    ceiling: Option<usize>,
) -> Arc<Flow> {
    flow_with_floor(key, identity, ceiling, None)
}

/// Like `flow_with_identity`; a new flow starts at least at `floor`.
fn flow_with_floor(
    key: String,
    identity: Option<String>,
    ceiling: Option<usize>,
    floor: Option<usize>,
) -> Arc<Flow> {
    let ceiling = ceiling
        .unwrap_or(RESOURCE_CEILING)
        .clamp(1, RESOURCE_CEILING);
    let mut registry = registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = Instant::now();
    registry.flows.retain(|_, (flow, used)| {
        Arc::strong_count(flow) > 1 || now.duration_since(*used) < FLOW_RETENTION
    });
    if let Some((flow, used)) = registry.flows.get_mut(&key) {
        *used = now;
        let flow = flow.clone();
        drop(registry);
        flow.set_ceiling(ceiling);
        return flow;
    }
    let learned = identity.as_deref().and_then(super::hints::start_limit);
    let start = match (learned, floor) {
        (Some(learned), Some(floor)) => Some(learned.max(floor)),
        (learned, floor) => learned.or(floor),
    };
    let flow = Arc::new(Flow::new(key.clone(), identity, start, ceiling));
    registry.flows.insert(key, (flow.clone(), now));
    flow
}

/// The flow of the connection serving `path` on `backend`.
pub fn flow_for(backend: &dyn crate::vfs::Backend, path: &str) -> Arc<Flow> {
    let floor = (backend.scheme() == crate::vfs::Scheme::Peer).then_some(PEER_START_LIMIT);
    flow_with_floor(
        backend.flow_key(path),
        Some(backend.flow_identity(path)),
        backend.transfer_ceiling(path),
        floor,
    )
}

/// The flow of the local volume holding `path`.
pub fn local_flow(path: &str) -> Arc<Flow> {
    flow_for(&crate::vfs::LocalBackend::new("/"), path)
}

/// Congestion (the peer asks to slow down) or an ordinary failure. Backends
/// report rate limits and "too many requests/connections" as
/// `vfs::congestion_error`; timeouts count as congestion too. A full disk or
/// an exhausted storage quota (`StorageFull`, `QuotaExceeded`) is permanent,
/// not congestion. The text check covers older services and agents that only
/// send their message.
pub fn classify_error(error: &std::io::Error) -> OpOutcome {
    use std::io::ErrorKind;
    if crate::vfs::congestion_of(error).is_some()
        || matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock)
        || error
            .to_string()
            .to_ascii_lowercase()
            .contains("too many concurrent")
    {
        OpOutcome::Overload
    } else {
        OpOutcome::Failed
    }
}
