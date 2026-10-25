//! Concrete old hashes require the incoming verified payload's explicit
//! catalog and freshly verified immutable bytes. The release suite exercises every
//! selected triple against the incoming daemon and shared state.
use crate::mcp_launcher::{Installation,RuntimeContract,os};
use crate::mcp_launcher::core::catalog::ReleaseCatalog;
use serde::Serialize;

#[derive(Debug,Serialize)]
pub struct CompatibilityRejection {pub runtime_sha256:String,pub reason:String}
#[derive(Debug,Default,Serialize)]
pub struct CompatibilitySelection {
    pub supported:Vec<RuntimeContract>,
    pub rejected:Vec<CompatibilityRejection>,
}
/// No unversioned image, directory scan, version heuristic or private copy is
/// admitted. Candidates come exclusively from the confirmed old manifest.
pub fn compatible_runtimes(previous:&Installation,incoming_sha256:&str,catalog:&ReleaseCatalog)->CompatibilitySelection {
    let mut candidates=vec![RuntimeContract {
        runtime_sha256:previous.manifest.runtime_sha256.clone(),
        ipc_contract_id:previous.manifest.ipc_contract_id.clone(),
        shared_state_contract_id:previous.manifest.shared_state_contract_id.clone(),
    }];
    candidates.extend(previous.manifest.supported_runtimes.clone());
    let mut seen=std::collections::BTreeSet::new();
    let mut selection=CompatibilitySelection::default();
    for runtime in candidates {
        if runtime.runtime_sha256==incoming_sha256 || !seen.insert(runtime.runtime_sha256.clone()) {continue;}
        let refusal=if !catalog.allows_runtime(&runtime) {
            Some("runtime hash/triple is not explicitly allowed by the incoming release catalog".into())
        } else {
            let path=previous.root.join("mcp/runtimes").join(&runtime.runtime_sha256).join(os::CLI_NAME);
            os::open_verified(&previous.root,&path,&runtime.runtime_sha256).err()
        };
        if let Some(reason)=refusal {
            selection.rejected.push(CompatibilityRejection {runtime_sha256:runtime.runtime_sha256,reason});
        } else {selection.supported.push(runtime);}
    }
    selection
}

/// Historical starter hashes come from incoming release policy only. The
/// installed allowlist is never carried forward as an authorization source.
pub fn compatible_launchers(root:&std::path::Path,catalog:&ReleaseCatalog)->Vec<String> {
    catalog.launchers.iter().cloned().filter(|hash| {
        let path=root.join("mcp/launchers").join(hash).join(os::LAUNCHER_NAME);
        os::open_verified(root,&path,hash).is_ok()
    }).collect()
}
