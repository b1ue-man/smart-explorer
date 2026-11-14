//! Login grants that follow „Bildschirm steuern: Erlauben“ (E21). The local
//! grant channel carries only devices; the account comes from the operating
//! system (Linux `SO_PEERCRED`, Windows client token), never from the wire.
//! The broker keeps a ledger of the grants it set itself, so administrative
//! grants stay exactly as an administrator wrote them.
use super::{
    gateway_types::{GatewayNetwork, GatewayTarget},
    model::{LoginRights, PeerGrant, Policy},
};
use serde::{Deserialize, Serialize};

pub const SELF_FORMAT: u32 = 1;
/// Steering devices one account may hold; keeps one request under `MAX_CONTROL`.
pub const MAX_SELF_GRANTS: usize = 64;
/// `Policy::validate` allows this many grants in total.
pub const MAX_POLICY_GRANTS: usize = 256;
pub const LEDGER_FILE: &str = "self-grants.json";
pub const MAX_LEDGER: usize = 64 * 1024;
/// A right without an end: ten years, renewed once fewer than five remain.
pub const OPEN_END: u64 = 10 * 365 * 24 * 3600;
pub const RENEW_BELOW: u64 = 5 * 365 * 24 * 3600;

/// One device that may steer this screen now, as the worker knows it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteeringGrant {
    pub peer_id: String,
    pub public_key: String,
    /// End of the steering right (Unix seconds); `None` = without end.
    pub valid_until: Option<u64>,
}

/// One request on the grant channel. No variant names an account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SelfRequest {
    Status,
    Grants {
        format: u32,
        grants: Vec<SteeringGrant>,
    },
    Network {
        network: GatewayNetwork,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelfReply {
    pub format: u32,
    /// `ok` or `refused`.
    pub state: String,
    pub reason: Option<String>,
    /// The release of the serving broker.
    pub release: String,
    /// The account the broker identified (Linux UID, Windows SID): the
    /// `target_user` a viewer names for this host's grants.
    pub account: String,
    pub policy_revision: u64,
    /// Peer IDs of this account's own self-set grants.
    pub own_grants: Vec<String>,
    /// Every other grant of the policy (administrative or another account's).
    pub other_grants: usize,
    /// The public machine target the gateway published, when it runs.
    pub target: Option<GatewayTarget>,
}

impl SelfReply {
    pub fn refused(account: &str, reason: impl ToString) -> Self {
        Self {
            format: SELF_FORMAT,
            state: "refused".into(),
            reason: Some(reason.to_string().chars().take(1024).collect()),
            release: env!("CARGO_PKG_VERSION").into(),
            account: account.into(),
            policy_revision: 0,
            own_grants: Vec::new(),
            other_grants: 0,
            target: None,
        }
    }
}

/// What the calling account can do at the console itself (step 2 (d)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConsoleAbilities {
    pub before_login: bool,
    pub locked: bool,
    pub uac: bool,
    pub sas: bool,
    /// The account works at this machine's console now (Linux: a session
    /// on seat0 or root; Windows: an interactive logon). Only then may it
    /// add a device; without it (e.g. an SSH-only account) its existing
    /// grants can only be kept, renewed or removed.
    pub console: bool,
}

impl ConsoleAbilities {
    pub fn rights(&self) -> LoginRights {
        LoginRights {
            screen_view: true,
            screen_control: true,
            login: true,
            before_login: self.before_login,
            locked: self.locked,
            uac: self.uac,
            sas: self.sas,
        }
    }
}

/// A grant the broker set for `account` on its channel.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerEntry {
    pub account: String,
    pub peer_id: String,
    pub public_key: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    pub format: u32,
    pub entries: Vec<LedgerEntry>,
}

impl Ledger {
    pub fn validate(&self) -> Result<(), &'static str> {
        if (self.format != SELF_FORMAT && !self.entries.is_empty())
            || self.entries.len() > MAX_POLICY_GRANTS
            || self
                .entries
                .iter()
                .any(|entry| !valid_account(&entry.account) || entry.peer_id.len() > 256)
        {
            return Err("invalid login self-grant ledger");
        }
        Ok(())
    }
    fn owns(&self, account: &str, grant: &PeerGrant) -> bool {
        grant.target_users.len() == 1
            && grant.target_users[0] == account
            && self.entries.iter().any(|entry| {
                entry.account == account
                    && entry.peer_id == grant.peer_id
                    && entry.public_key == grant.public_key
            })
    }
    /// Peer IDs of `account`'s self-set grants that `policy` holds.
    pub fn own_grants(&self, policy: &Policy, account: &str) -> Vec<String> {
        policy
            .grants
            .iter()
            .filter(|grant| self.owns(account, grant))
            .map(|grant| grant.peer_id.clone())
            .collect()
    }
    pub fn holds(&self, account: &str) -> bool {
        self.entries.iter().any(|entry| entry.account == account)
    }
}

/// An account string the broker accepts: a Linux UID or a Windows SID.
pub fn valid_account(account: &str) -> bool {
    !account.is_empty()
        && account.len() <= 256
        && account
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// The result of one grant request.
#[derive(Debug)]
pub struct Merged {
    /// The new policy; `None` when nothing changed (no revision step).
    pub policy: Option<Policy>,
    /// Written before the policy: every entry that may be in force.
    pub ledger_before: Ledger,
    /// Written after the policy: exactly the self-set grants in force.
    pub ledger_after: Ledger,
}

fn same_grants(left: &[PeerGrant], right: &[PeerGrant]) -> bool {
    let canonical = |grants: &[PeerGrant]| {
        let mut values: Vec<String> = grants
            .iter()
            .filter_map(|grant| serde_json::to_string(grant).ok())
            .collect();
        values.sort();
        values
    };
    left.len() == right.len() && canonical(left) == canonical(right)
}

/// One entry per device, in `Ord` order. The share runtime may name a device
/// twice (a Direct grant and a room membership, each with its own end), and
/// the broker refuses a request that names a device twice, so the later end
/// wins (`None`, without an end, is the latest); for the same end the smaller
/// key, so the choice is stable.
pub fn one_per_device(mut grants: Vec<SteeringGrant>) -> Vec<SteeringGrant> {
    let end = |grant: &SteeringGrant| grant.valid_until.unwrap_or(u64::MAX);
    grants.sort_by(|left, right| {
        left.peer_id
            .cmp(&right.peer_id)
            .then_with(|| end(right).cmp(&end(left)))
            .then_with(|| left.public_key.cmp(&right.public_key))
    });
    grants.dedup_by(|next, kept| next.peer_id == kept.peer_id);
    grants.sort();
    grants
}

fn check_request(requested: &[SteeringGrant]) -> Result<(), &'static str> {
    if requested.len() > MAX_SELF_GRANTS {
        return Err("too many steering devices for one account");
    }
    let mut peers = std::collections::BTreeSet::new();
    for grant in requested {
        if grant.peer_id.is_empty()
            || grant.peer_id.len() > 256
            || !peers.insert(grant.peer_id.as_str())
            || grant.public_key.parse::<iroh::PublicKey>().is_err()
        {
            return Err("invalid steering device in login grant request");
        }
    }
    Ok(())
}

/// Replaces `account`'s self-set grants by `requested`. Grants the ledger
/// does not attribute to `account` stay untouched; a requested device that
/// another grant already names keeps that grant (administrator first).
pub fn merge(
    policy: &Policy,
    ledger: &Ledger,
    account: &str,
    requested: &[SteeringGrant],
    abilities: &ConsoleAbilities,
    now: u64,
) -> Result<Merged, &'static str> {
    if !valid_account(account) {
        return Err("login grant channel caller has no valid account");
    }
    ledger.validate()?;
    check_request(requested)?;
    let previous: Vec<&PeerGrant> = policy
        .grants
        .iter()
        .filter(|grant| ledger.owns(account, grant))
        .collect();
    let mut grants: Vec<PeerGrant> = policy
        .grants
        .iter()
        .filter(|grant| !ledger.owns(account, grant))
        .cloned()
        .collect();
    let mut sorted: Vec<&SteeringGrant> = requested.iter().collect();
    sorted.sort();
    let mut own = Vec::new();
    for device in sorted {
        if device.valid_until.is_some_and(|until| until <= now)
            || grants.iter().any(|grant| grant.peer_id == device.peer_id)
        {
            continue;
        }
        let before = previous
            .iter()
            .find(|grant| grant.peer_id == device.peer_id && grant.public_key == device.public_key);
        if before.is_none() && !abilities.console {
            continue;
        }
        let expires_at = match device.valid_until {
            Some(until) => until,
            None => before
                .map(|grant| grant.expires_at)
                .filter(|end| *end >= now.saturating_add(RENEW_BELOW))
                .unwrap_or_else(|| now.saturating_add(OPEN_END)),
        };
        let not_before = before
            .map(|grant| grant.not_before)
            .filter(|start| *start < expires_at)
            .unwrap_or(now);
        grants.push(PeerGrant {
            peer_id: device.peer_id.clone(),
            public_key: device.public_key.clone(),
            target_users: vec![account.to_string()],
            not_before,
            expires_at,
            rights: abilities.rights(),
        });
        own.push(LedgerEntry {
            account: account.into(),
            peer_id: device.peer_id.clone(),
            public_key: device.public_key.clone(),
        });
    }
    if grants.len() > MAX_POLICY_GRANTS {
        return Err("login policy grant limit reached");
    }
    let others: Vec<LedgerEntry> = ledger
        .entries
        .iter()
        .filter(|entry| entry.account != account)
        .cloned()
        .collect();
    let mut after = Ledger {
        format: SELF_FORMAT,
        entries: others.iter().cloned().chain(own).collect(),
    };
    after.entries.sort();
    after.entries.dedup();
    let mut before = after.clone();
    before.entries.extend(
        ledger
            .entries
            .iter()
            .filter(|entry| entry.account == account)
            .cloned(),
    );
    before.entries.sort();
    before.entries.dedup();
    let changed = !same_grants(&policy.grants, &grants);
    let policy = if changed {
        let next = Policy {
            revision: policy
                .revision
                .checked_add(1)
                .ok_or("login policy revision exhausted")?,
            grants,
            ..policy.clone()
        };
        next.validate()?;
        Some(next)
    } else {
        None
    };
    Ok(Merged {
        policy,
        ledger_before: before,
        ledger_after: after,
    })
}

/// Whether the installed network record must be replaced by `wanted`.
pub fn network_differs(installed: Option<&GatewayNetwork>, wanted: &GatewayNetwork) -> bool {
    installed != Some(wanted)
}

#[cfg(test)]
#[path = "self_grants_task_tests.rs"]
mod self_grants_task_tests;
