//! The manifest is an allowlist, never a command or downloader configuration.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const SCHEMA: u32 = 1;
pub const LAUNCHER_PROTOCOL: u32 = 1;
// These IDs freeze token/root/credential/cancellation/result semantics, not
// merely JSON shape. An incompatible change needs a new ID or an adapter.
pub const IPC_CONTRACT: &str = "se-mcp-ipc-2026-10-25-v1";
pub const STATE_CONTRACT: &str = "se-mcp-state-2026-10-25-v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeContract {
    pub runtime_sha256: String,
    pub ipc_contract_id: String,
    pub shared_state_contract_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub installation_id: String,
    pub committed_transaction_id: String,
    pub product_version: String,
    pub generation: u64,
    pub runtime_sha256: String,
    pub runtime_file: String,
    pub launcher_protocol: u32,
    pub launcher_sha256: String,
    pub launcher_product_version: String,
    pub allowed_launcher_hashes: Vec<String>,
    pub ipc_contract_id: String,
    pub shared_state_contract_id: String,
    pub supported_runtimes: Vec<RuntimeContract>,
    pub compatibility_catalog_sha256: String,
}

pub fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn is_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

impl Manifest {
    pub fn validate(&self, executable_name: &str) -> Result<(), String> {
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() > 256 * 1024 {
            return Err("runtime_manifest_exceeds_record_budget".into());
        }
        if self.schema != SCHEMA || self.launcher_protocol != LAUNCHER_PROTOCOL {
            return Err("runtime_manifest_schema_unsupported".into());
        }
        if !is_id(&self.installation_id)
            || !is_id(&self.committed_transaction_id)
            || !is_id(&self.product_version)
            || !is_id(&self.launcher_product_version)
            || self.generation == 0
            || !is_sha256(&self.runtime_sha256)
            || !is_sha256(&self.launcher_sha256)
            || !is_sha256(&self.compatibility_catalog_sha256)
            || !is_id(&self.ipc_contract_id)
            || !is_id(&self.shared_state_contract_id)
        {
            return Err("runtime_manifest_invalid_identity_or_contract".into());
        }
        let expected = format!("mcp/runtimes/{}/{executable_name}", self.runtime_sha256);
        if self.runtime_file != expected {
            return Err("runtime_manifest_path_invalid".into());
        }
        let mut launchers = BTreeSet::new();
        if self.allowed_launcher_hashes.is_empty() {
            return Err("runtime_manifest_launcher_allowlist_invalid".into());
        }
        for hash in &self.allowed_launcher_hashes {
            if !is_sha256(hash) || !launchers.insert(hash) {
                return Err("runtime_manifest_duplicate_or_invalid_launcher".into());
            }
        }
        if !launchers.contains(&self.launcher_sha256) {
            return Err("runtime_manifest_current_launcher_not_allowed".into());
        }
        let mut runtimes = BTreeSet::new();
        runtimes.insert(&self.runtime_sha256);
        for runtime in &self.supported_runtimes {
            if !is_sha256(&runtime.runtime_sha256)
                || !runtimes.insert(&runtime.runtime_sha256)
                || !is_id(&runtime.ipc_contract_id)
                || !is_id(&runtime.shared_state_contract_id)
            {
                return Err("runtime_manifest_duplicate_or_unimplemented_contract".into());
            }
        }
        Ok(())
    }

    pub fn permits(&self, runtime: &RuntimeContract) -> bool {
        implements_contract_pair(&runtime.ipc_contract_id, &runtime.shared_state_contract_id)
            && ((runtime.runtime_sha256 == self.runtime_sha256
                && runtime.ipc_contract_id == self.ipc_contract_id
                && runtime.shared_state_contract_id == self.shared_state_contract_id)
                || self.supported_runtimes.contains(runtime))
    }
}

/// This is a semantic offer by this build. It is not inferred from a version,
/// Ping, schema hash or successful JSON decoding. Add a pair only alongside
/// its actual adapter and the release-suite compatibility cases.
pub fn implements_contract_pair(ipc: &str, state: &str) -> bool {
    ipc == IPC_CONTRACT && state == STATE_CONTRACT
}
