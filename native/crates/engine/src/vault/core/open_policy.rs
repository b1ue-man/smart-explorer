//! Identity-bound mount consent. Encryption is not a root-confinement proof.
use serde::{Deserialize, Serialize};

pub const POLICY_KEY: &str = "open_policy_v1";
pub const ROOT_TRUST_REQUIRED: &str = "vault_root_trust_required";
pub const MOUNT_RUNTIME_MISSING: &str = "vault_mount_runtime_missing";

/// Exact storage identity observed when consent was given. No normalization of
/// literal provider names, roots or account identities is permitted here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootBinding {
    pub site: String,
    pub vault: String,
    pub location: String,
    pub backend: String,
    pub namespace: String,
    pub account: String,
    pub root: String,
    pub object: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenPolicy {
    pub trusted_root: Option<RootBinding>,
    /// Independent explicit consent; never implied by root trust.
    pub cleartext_copy: Option<RootBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountPolicy {
    pub binding: RootBinding,
    pub enforced: bool,
    pub trusted: bool,
    pub cleartext_copy: bool,
    pub read_only: bool,
}

impl MountPolicy {
    pub fn allowed(&self) -> bool {
        self.enforced || self.trusted
    }
}

pub fn stored_policy(
    record: &super::registry_types::SiteRecord,
) -> Result<OpenPolicy, String> {
    let mut values = record.extra.iter().filter(|(key, _)| key == POLICY_KEY);
    let Some((_, value)) = values.next() else {
        return Ok(OpenPolicy::default());
    };
    if values.next().is_some() {
        return Err("Mehrdeutige Tresor-Öffnungsrichtlinie".into());
    }
    serde_json::from_str(value).map_err(|_| "Ungültige Tresor-Öffnungsrichtlinie".into())
}

pub fn store_policy(
    record: &mut super::registry_types::SiteRecord,
    policy: &OpenPolicy,
) -> Result<(), String> {
    let value = serde_json::to_string(policy).map_err(|error| error.to_string())?;
    record.extra.retain(|(key, _)| key != POLICY_KEY);
    record.extra.push((POLICY_KEY.into(), value));
    Ok(())
}

#[cfg(test)]
#[path = "batches_open_policy_task_tests.rs"]
mod batches_open_policy_task_tests;
