//! Locally listening TCP services that „Bildschirm steuern: Erlauben“ shares
//! with the steering device (design 4.1/4.2, user rule r5: „wenn auf
//! localhost was offen ist wird das frei gegeben für andere geräte. per
//! default“). Platform-neutral: the OS adapters (`share::os_sockets`) supply
//! the listener rows; this module classifies them and decides exposure.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, SocketAddr};

/// Where a listener accepts connections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BindScope {
    /// 127.0.0.0/8, ::1 or ::ffff:127.0.0.0/104 only.
    LoopbackOnly,
    /// 0.0.0.0 or ::, therefore also loopback.
    Wildcard,
}

/// The account class of the listening process (design 4.2).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum OwnerClass {
    /// A person's account: Linux UID ≥ `UID_MIN`, Windows SID of a user.
    NormalUser { uid_or_sid: String },
    /// SYSTEM, LOCAL/NETWORK SERVICE, `NT SERVICE\*`, Linux UID < `UID_MIN`.
    System,
    /// Not determinable; counts as a system account for wildcard listeners.
    Unknown,
}

/// One listener row as the OS reports it (`share::os_sockets::listeners`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListenerRow {
    pub local: SocketAddr,
    pub pid: Option<u32>,
    pub owner: OwnerClass,
}

/// One listening port as the host offers it to a steering device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoopbackService {
    pub port: u16,
    pub scope: BindScope,
    pub owner: OwnerClass,
    #[serde(default)]
    pub process: Option<String>,
    #[serde(default)]
    pub pid: Option<u32>,
}
impl LoopbackService {
    /// Bounds of one received service entry (feed and `LocalServices`).
    pub(crate) fn validate(&self) -> Result<(), String> {
        let text_ok = |value: &str| value.len() <= 256 && !value.chars().any(char::is_control);
        let owner_ok = match &self.owner {
            OwnerClass::NormalUser { uid_or_sid } => !uid_or_sid.is_empty() && text_ok(uid_or_sid),
            OwnerClass::System | OwnerClass::Unknown => true,
        };
        if self.port == 0 || !owner_ok || !self.process.as_deref().is_none_or(text_ok) {
            return Err("invalid local service entry".into());
        }
        Ok(())
    }
}

/// At most this many services are reported; a host with more listeners
/// reports the lowest ports and says so (`truncated`).
pub(crate) const MAX_SERVICES: usize = 512;

/// The binding scope of one listener address; `None` for an address that
/// is bound to one non-loopback interface and therefore unreachable over
/// the host's loopback.
pub(crate) fn scope_of(address: IpAddr) -> Option<BindScope> {
    match address {
        IpAddr::V4(v4) if v4.is_loopback() => Some(BindScope::LoopbackOnly),
        IpAddr::V4(v4) if v4.is_unspecified() => Some(BindScope::Wildcard),
        IpAddr::V6(v6) if v6.is_loopback() => Some(BindScope::LoopbackOnly),
        IpAddr::V6(v6) if v6.is_unspecified() => Some(BindScope::Wildcard),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) if v4.is_loopback() => Some(BindScope::LoopbackOnly),
            _ => None,
        },
        IpAddr::V4(_) => None,
    }
}

/// Whether `service` is shared under steering (design 4.2): every
/// loopback-only listener of any account; a wildcard listener only of a
/// normal user account; never an excluded port.
pub(crate) fn exposable(service: &LoopbackService, excluded_ports: &BTreeSet<u16>) -> bool {
    if service.port == 0 || excluded_ports.contains(&service.port) {
        return false;
    }
    match service.scope {
        BindScope::LoopbackOnly => true,
        BindScope::Wildcard => matches!(service.owner, OwnerClass::NormalUser { .. }),
    }
}

/// Folds listener rows into one entry per port. A port with any wildcard
/// row is a wildcard service and takes that row's owner; loopback-only rows
/// keep their own owner. Rows of excluded processes (`skip_pid`) remove the
/// port entirely, because their service is se itself.
pub(crate) fn services_from(
    rows: &[ListenerRow],
    skip_pid: impl Fn(u32) -> bool,
    name: impl Fn(u32) -> Option<String>,
) -> Vec<LoopbackService> {
    let mut skipped = BTreeSet::new();
    let mut ports: BTreeMap<u16, LoopbackService> = BTreeMap::new();
    for row in rows {
        let port = row.local.port();
        if port == 0 {
            continue;
        }
        if row.pid.is_some_and(&skip_pid) {
            skipped.insert(port);
            continue;
        }
        let Some(scope) = scope_of(row.local.ip()) else {
            continue;
        };
        let candidate = LoopbackService {
            port,
            scope,
            owner: row.owner.clone(),
            process: row.pid.and_then(&name),
            pid: row.pid,
        };
        let replace = ports
            .get(&port)
            .is_none_or(|known| more_telling(known, &candidate));
        if replace {
            ports.insert(port, candidate);
        }
    }
    ports
        .into_values()
        .filter(|service| !skipped.contains(&service.port))
        .take(MAX_SERVICES)
        .collect()
}

/// A wildcard row decides the scope of its port; among rows of one scope a
/// row that names its process is kept.
fn more_telling(known: &LoopbackService, candidate: &LoopbackService) -> bool {
    let wider = known.scope == BindScope::LoopbackOnly && candidate.scope == BindScope::Wildcard;
    let named =
        known.scope == candidate.scope && known.process.is_none() && candidate.process.is_some();
    wider || named
}

/// The ports among `services` that a steering device may reach.
pub(crate) fn exposed_ports(
    services: &[LoopbackService],
    excluded_ports: &BTreeSet<u16>,
) -> BTreeSet<u16> {
    services
        .iter()
        .filter(|service| exposable(service, excluded_ports))
        .map(|service| service.port)
        .collect()
}

/// Program stems of se itself (release feed names); their listeners are
/// se-internal (T-L3) and never shared.
pub(crate) const SE_PROGRAM_STEMS: &[&str] = &[
    "se",
    "smart_explorer",
    "smart_explorer_updater",
    "se-mcp-launcher",
    "se-share-server",
    "se-agent",
];

/// Whether `image` is an installed se program next to `own_exe`.
pub(crate) fn is_se_image(image: &std::path::Path, own_exe: &std::path::Path) -> bool {
    let same_dir = image.parent().is_some() && image.parent() == own_exe.parent();
    let stem = image
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_ascii_lowercase);
    same_dir && stem.is_some_and(|stem| SE_PROGRAM_STEMS.contains(&stem.as_str()))
}
