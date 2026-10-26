//! Pure lifecycle records. No PID alone authorizes recovery or retention.
use super::manifest::{is_id, is_sha256, Manifest, RuntimeContract};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub started: String,
    pub image: String,
    #[serde(default)]
    pub executable_object: Option<ExecutableObject>,
}

/// Stable identity of the mapped Linux executable, independent of its name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutableObject {
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopIdentity {
    pub file_id: String,
    pub modified: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Prepared,
    Stopping,
    Committed,
    RolledBack,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationTarget {
    pub file: String,
    pub previous_sha256: Option<String>,
    pub target_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateState {
    pub schema: u32,
    pub installation_id: String,
    pub transaction_id: String,
    pub target_manifest_sha256: String,
    pub owner: ProcessIdentity,
    pub phase: Phase,
    pub target: Manifest,
    pub previous: Manifest,
    pub stop: Option<StopIdentity>,
    pub legacy_stop: Option<StopIdentity>,
    pub installation_targets: Vec<InstallationTarget>,
    pub retire_managed: bool,
}

impl UpdateState {
    pub fn validate(&self, executable_name: &str) -> Result<(), String> {
        self.target.validate(executable_name)?;
        self.previous.validate(executable_name)?;
        let mut files = std::collections::BTreeSet::new();
        for target in &self.installation_targets {
            if target.file.is_empty()
                || target.file == "."
                || target.file == ".."
                || target
                    .file
                    .bytes()
                    .any(|b| !b.is_ascii_alphanumeric() && !b" -_.".contains(&b))
                || !files.insert(&target.file)
                || !is_sha256(&target.target_sha256)
                || target
                    .previous_sha256
                    .as_ref()
                    .is_some_and(|h| !is_sha256(h))
            {
                return Err("update_target_proof_invalid".into());
            }
        }
        if self.schema != 1
            || !is_id(&self.transaction_id)
            || !is_sha256(&self.target_manifest_sha256)
            || self.installation_id != self.target.installation_id
            || self.installation_id != self.previous.installation_id
            || self.target.committed_transaction_id != self.transaction_id
            || self.target.generation <= self.previous.generation
            || self.target_manifest_sha256
                != format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&self.target).map_err(|e| e.to_string())?)
                )
            || self.owner.pid == 0
            || self.owner.started.is_empty()
            || self.owner.image.is_empty()
        {
            return Err("update_state_invalid".into());
        }
        Ok(())
    }
    pub fn blocks_work(&self) -> bool {
        matches!(self.phase, Phase::Prepared | Phase::Stopping)
    }
    pub fn retains(&self, installation_id: &str, runtime: &RuntimeContract) -> bool {
        !self.retire_managed
            && self.phase == Phase::Stopping
            && self.stop.is_some()
            && self.installation_id == installation_id
            && self.target.permits(runtime)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeHello {
    pub installation_id: String,
    pub runtime_sha256: String,
    pub ipc_contract_id: String,
    pub shared_state_contract_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeHelloReply {
    pub accepted: bool,
    pub daemon_generation: String,
    pub ipc_contract_id: String,
    pub shared_state_contract_id: String,
    pub error_code: Option<String>,
}
