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
/// A local volume that is flash or cannot be told (network share, FUSE,
/// a Windows volume spanning several disks) starts at twice the cores, within these bounds (M7,
/// 2026-10-28): small files reach their steady rate at once and a large
/// file gets the extra threads it plans for (`UNBUFFERED_EXTRAS` = 3 plus
/// its own) from its first byte. A rotating disk keeps the general start of
/// 2; the controller grows or shrinks every start from there.
const LOCAL_START_MIN: usize = 8;
const LOCAL_START_MAX: usize = 16;

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
    flow_with_floor(key, identity, ceiling, || None)
}

/// Like `flow_with_identity`; a new flow starts at least at `floor()`
/// (asked only when the flow is created).
fn flow_with_floor(
    key: String,
    identity: Option<String>,
    ceiling: Option<usize>,
    floor: impl FnOnce() -> Option<usize>,
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
    let start = match (learned, floor()) {
        (Some(learned), Some(floor)) => Some(learned.max(floor)),
        (learned, floor) => learned.or(floor),
    };
    let flow = Arc::new(Flow::new(key.clone(), identity, start, ceiling));
    registry.flows.insert(key, (flow.clone(), now));
    flow
}

/// The flow of the connection serving `path` on `backend`.
pub fn flow_for(backend: &dyn crate::vfs::Backend, path: &str) -> Arc<Flow> {
    let scheme = backend.scheme();
    flow_with_floor(
        backend.flow_key(path),
        Some(backend.flow_identity(path)),
        backend.transfer_ceiling(path),
        || start_floor(scheme, path),
    )
}

/// The least a new flow of `scheme` serving `path` starts at; `None` keeps
/// the general start (or the learned one).
fn start_floor(scheme: crate::vfs::Scheme, path: &str) -> Option<usize> {
    match scheme {
        crate::vfs::Scheme::Peer => Some(PEER_START_LIMIT),
        crate::vfs::Scheme::Local => local_start(local_rotational(path)),
        // G3-09: remote servers start with several parallel transfers; the
        // flow clamps the start to the backend's `transfer_ceiling`.
        crate::vfs::Scheme::Sftp | crate::vfs::Scheme::Smb | crate::vfs::Scheme::Webdav => {
            Some(REMOTE_START)
        }
        crate::vfs::Scheme::Ftp | crate::vfs::Scheme::GDrive => Some(SESSION_START),
        crate::vfs::Scheme::Vault => None,
    }
}

/// Start of an SFTP, SMB or WebDAV flow (G3-09).
const REMOTE_START: usize = 8;
/// Start of an FTP or Drive flow: each transfer costs a login or a create
/// quota there (G3-09).
const SESSION_START: usize = 4;

/// The start of a local volume's flow: `None` (general start) for a disk
/// known to rotate, otherwise twice the cores within the local bounds.
fn local_start(rotational: Option<bool>) -> Option<usize> {
    if rotational == Some(true) {
        return None;
    }
    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    Some(
        cores
            .saturating_mul(2)
            .clamp(LOCAL_START_MIN, LOCAL_START_MAX),
    )
}

/// Whether the drive under the local `path` rotates (Linux `/sys`
/// `queue/rotational`, Windows seek-penalty query).
fn local_rotational(path: &str) -> Option<bool> {
    crate::transfer::platform::volume_is_rotational(std::path::Path::new(path))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_speed_task_local_flows_start_wide_except_on_rotating_disks() {
        assert_eq!(local_start(Some(true)), None);
        for rotational in [Some(false), None] {
            let start = local_start(rotational).expect("flash or unknown starts wide");
            assert!((LOCAL_START_MIN..=LOCAL_START_MAX).contains(&start));
        }
        assert_eq!(
            start_floor(crate::vfs::Scheme::Peer, "/"),
            Some(PEER_START_LIMIT)
        );
        assert_eq!(
            start_floor(crate::vfs::Scheme::Sftp, "/"),
            Some(REMOTE_START)
        );
        assert_eq!(
            start_floor(crate::vfs::Scheme::Ftp, "/"),
            Some(SESSION_START)
        );
        assert_eq!(start_floor(crate::vfs::Scheme::Vault, "/"), None);
    }
}
