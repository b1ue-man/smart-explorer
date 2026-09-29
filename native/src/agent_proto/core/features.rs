//! Capability labels carried by the display-only `HelloOk` version, e.g.
//! `0.5.0+sync-links-v1.credit-v1.stage-v1.slots-64.batch-v1 worker`.
//! Old peers send no label; a client then keeps every former behaviour.
use super::credit::CREDIT_REQUEST_LIMIT;

pub const LABEL_LINKS: &str = "sync-links-v1";
/// Credit flow control (`Frame::Credit`) and the congestion marker.
pub const LABEL_CREDIT: &str = "credit-v1";
/// `BatchPut` / `BatchGet`.
pub const LABEL_BATCH: &str = "batch-v1";
/// `CopyToStage`, `CreateDir`, `DiscardStage`.
pub const LABEL_STAGE: &str = "stage-v1";
/// Prefix of the label announcing the admitted concurrent requests.
const LABEL_SLOTS: &str = "slots-";

/// Admitted requests of servers without `credit-v1` (agent, service).
const LEGACY_AGENT_SLOTS: usize = 8;
const LEGACY_SERVICE_SLOTS: usize = 16;
/// Requests kept free for browsing next to transfers (research §2: two on
/// an agent connection, four on the service that also serves mounts).
const AGENT_BROWSE_RESERVE: usize = 2;
const SERVICE_BROWSE_RESERVE: usize = 4;

/// The version text a server of this build announces.
pub fn server_version(batch: bool) -> String {
    server_version_with(batch, CREDIT_REQUEST_LIMIT)
}

/// The version text with `slots` admitted requests (at most the credit
/// limit, which bounds the connection's buffer budget).
pub fn server_version_with(batch: bool, slots: usize) -> String {
    let slots = slots.clamp(1, CREDIT_REQUEST_LIMIT);
    let mut version = format!(
        "{}+{LABEL_LINKS}.{LABEL_CREDIT}.{LABEL_STAGE}.{LABEL_SLOTS}{slots}",
        env!("CARGO_PKG_VERSION")
    );
    if batch {
        version.push('.');
        version.push_str(LABEL_BATCH);
    }
    version
}

/// Slots the background service announces: its own limit, or fewer when the
/// peer behind it admits fewer transfer operations (plus the browsing
/// reserve), so clients never plan more transfers than the peer takes.
pub fn service_slots(peer_ceiling: Option<usize>) -> usize {
    peer_ceiling.map_or(CREDIT_REQUEST_LIMIT, |ceiling| {
        ceiling
            .saturating_add(SERVICE_BROWSE_RESERVE)
            .min(CREDIT_REQUEST_LIMIT)
    })
}

/// What a server announced in its `HelloOk` version.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServerFeatures {
    pub link_aware_hash: bool,
    pub credit: bool,
    pub batch: bool,
    pub stage: bool,
    /// Requests the server admits at once, when it announced them.
    pub slots: Option<usize>,
    /// The background service (announces itself with a trailing `worker`).
    pub service: bool,
}

impl ServerFeatures {
    pub fn parse(version: &str) -> Self {
        let mut words = version.split_whitespace();
        let labels = words
            .next()
            .and_then(|text| text.split_once('+'))
            .map_or("", |(_, labels)| labels);
        let mut features = Self {
            service: words.next() == Some("worker"),
            ..Self::default()
        };
        for label in labels.split('.') {
            match label {
                LABEL_LINKS => features.link_aware_hash = true,
                LABEL_CREDIT => features.credit = true,
                LABEL_BATCH => features.batch = true,
                LABEL_STAGE => features.stage = true,
                other => {
                    if let Some(slots) = other.strip_prefix(LABEL_SLOTS) {
                        features.slots = slots.parse().ok().filter(|slots| *slots > 0);
                    }
                }
            }
        }
        features
    }

    /// Batches need credit flow control for their streams.
    pub fn batches(&self) -> bool {
        self.batch && self.credit
    }

    /// Requests one connection admits at once.
    pub fn admitted(&self) -> usize {
        self.slots.unwrap_or(if self.service {
            LEGACY_SERVICE_SLOTS
        } else {
            LEGACY_AGENT_SLOTS
        })
    }

    /// Requests transfers may occupy on one connection; the rest stays free
    /// for browsing and mounts.
    pub fn transfer_slots(&self) -> usize {
        let reserve = if self.service {
            SERVICE_BROWSE_RESERVE
        } else {
            AGENT_BROWSE_RESERVE
        };
        self.admitted().saturating_sub(reserve).max(1)
    }
}
