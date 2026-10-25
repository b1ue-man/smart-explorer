//! Explicit release policy carried by the SHA-verified incoming runtime.
use super::manifest::{RuntimeContract,is_id,is_sha256};
use serde::{Serialize,Deserialize};
use sha2::{Digest,Sha256};
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseCatalog {
    pub schema:u32,pub product_version:String,
    pub runtimes:Vec<RuntimeContract>,pub launchers:Vec<String>,
}
// First managed release: no historical managed runtime or starter exists.
// Future approvals require exact hashes and release-suite compatibility proof.
const HISTORICAL_RUNTIME_APPROVALS:&[(&str,&str,&str)]=&[];
const HISTORICAL_STARTER_APPROVALS:&[&str]=&[];
impl ReleaseCatalog {
    pub fn current()->Self {
        Self {schema:1,product_version:env!("CARGO_PKG_VERSION").into(),runtimes:HISTORICAL_RUNTIME_APPROVALS.iter().map(|(hash,ipc,state)|RuntimeContract {
            runtime_sha256:(*hash).into(),ipc_contract_id:(*ipc).into(),shared_state_contract_id:(*state).into(),
        }).collect(),launchers:HISTORICAL_STARTER_APPROVALS.iter().map(|hash|(*hash).into()).collect()}
    }
    pub fn validate(&self,version:&str)->Result<(),String> {
        let mut runtimes=std::collections::BTreeSet::new();let mut launchers=std::collections::BTreeSet::new();
        if self.schema!=1 || self.product_version!=version
            || self.runtimes.iter().any(|r|!is_sha256(&r.runtime_sha256)||!is_id(&r.ipc_contract_id)||!is_id(&r.shared_state_contract_id)||!runtimes.insert(&r.runtime_sha256))
            || self.launchers.iter().any(|hash|!is_sha256(hash)||!launchers.insert(hash)) {
            return Err("incoming_release_compatibility_catalog_invalid".into());
        }
        Ok(())
    }
    pub fn allows_runtime(&self,runtime:&RuntimeContract)->bool {self.runtimes.contains(runtime)}
    pub fn allows_launcher(&self,hash:&str)->bool {self.launchers.iter().any(|allowed|allowed==hash)}
    pub fn sha256(&self)->String {
        let mut digest=Sha256::new();digest.update(b"se-mcp-compatibility-catalog-v1");digest.update(self.schema.to_be_bytes());
        fn field(digest:&mut Sha256,value:&str) {digest.update((value.len() as u64).to_be_bytes());digest.update(value.as_bytes());}
        field(&mut digest,&self.product_version);digest.update((self.runtimes.len() as u64).to_be_bytes());
        for runtime in &self.runtimes {field(&mut digest,&runtime.runtime_sha256);field(&mut digest,&runtime.ipc_contract_id);field(&mut digest,&runtime.shared_state_contract_id);}
        digest.update((self.launchers.len() as u64).to_be_bytes());for launcher in &self.launchers {field(&mut digest,launcher);}
        format!("{:x}",digest.finalize())
    }
}
