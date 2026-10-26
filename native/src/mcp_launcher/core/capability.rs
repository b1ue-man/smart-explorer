//! Capability comes from the exact verified binary, never a version heuristic.
use super::manifest::{IPC_CONTRACT, LAUNCHER_PROTOCOL, STATE_CONTRACT};
use serde::{Deserialize, Serialize};
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeProbe {
    pub schema: u32,
    pub component: String,
    pub product_version: String,
    pub launcher_protocol: u32,
    pub admission_protocol: u32,
    pub lifecycle_protocol: u32,
    pub update_protocol: u32,
    pub ipc_contract_id: String,
    pub shared_state_contract_id: String,
    pub compatibility_catalog_sha256: String,
    pub compatibility_catalog: super::catalog::ReleaseCatalog,
}
impl RuntimeProbe {
    pub fn current() -> Self {
        Self {
            schema: 1,
            component: "se-runtime".into(),
            product_version: env!("CARGO_PKG_VERSION").into(),
            launcher_protocol: LAUNCHER_PROTOCOL,
            admission_protocol: 1,
            lifecycle_protocol: 1,
            update_protocol: 1,
            ipc_contract_id: IPC_CONTRACT.into(),
            shared_state_contract_id: STATE_CONTRACT.into(),
            compatibility_catalog_sha256: super::catalog::ReleaseCatalog::current().sha256(),
            compatibility_catalog: super::catalog::ReleaseCatalog::current(),
        }
    }
    pub fn validate(&self, version: &str) -> Result<(), String> {
        if self.schema != 1
            || self.component != "se-runtime"
            || self.product_version != version
            || self.launcher_protocol != LAUNCHER_PROTOCOL
            || self.admission_protocol != 1
            || self.lifecycle_protocol != 1
            || self.update_protocol != 1
            || !super::manifest::is_id(&self.ipc_contract_id)
            || !super::manifest::is_id(&self.shared_state_contract_id)
            || self.compatibility_catalog_sha256 != self.compatibility_catalog.sha256()
        {
            return Err("runtime_capability_contract_or_product_version_mismatch".into());
        }
        self.compatibility_catalog.validate(version)?;
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() >= 256 * 1024 {
            return Err("runtime_capability_probe_exceeds_record_budget".into());
        }
        Ok(())
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LauncherProbe {
    pub schema: u32,
    pub component: String,
    pub product_version: String,
    pub launcher_protocol: u32,
}
impl LauncherProbe {
    pub fn current() -> Self {
        Self {
            schema: 1,
            component: "se-mcp-launcher".into(),
            product_version: env!("CARGO_PKG_VERSION").into(),
            launcher_protocol: LAUNCHER_PROTOCOL,
        }
    }
    pub fn validate(&self, version: &str) -> Result<(), String> {
        if self.schema != 1
            || self.component != "se-mcp-launcher"
            || self.product_version != version
            || self.launcher_protocol != LAUNCHER_PROTOCOL
        {
            return Err("launcher_capability_contract_or_product_version_mismatch".into());
        }
        Ok(())
    }
}
