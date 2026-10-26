//! Called only with expected hashes from the verified release/install path.
use crate::mcp_launcher::{os,Installation,Manifest};
use crate::mcp_launcher::installation::{atomic_json,verify_handle};
use std::fs::File;
use std::io::Write;
use std::path::{Path,PathBuf};

pub struct GenerationInput {
    pub installation_id:String,
    pub product_version:String,
    pub runtime_path:PathBuf,
    pub runtime_sha256:String,
    pub launcher_path:PathBuf,
    pub launcher_sha256:String,
}
/// Provision immutable files without changing the confirmed pointer.
pub fn stage_generation(root:&Path,input:&GenerationInput)->Result<Manifest,String> {
    if root.join("mcp/uninstall.v1.json").exists() {return Err("installation_uninstall_in_progress".into());}
    let previous=if root.join("mcp/current.v1.json").exists() {Some(Installation::load(root)?)} else {None};
    let history=if previous.is_none() && root.join("mcp/legacy.v1.json").exists() {
        let path=root.join("mcp/legacy.v1.json");os::validate_tree(root,&path)?;
        let history:Manifest=crate::mcp_launcher::installation::read_json(&path)?;history.validate(os::CLI_NAME)?;Some(history)
    }else{None};
    if history.as_ref().is_some_and(|h|h.installation_id!=input.installation_id) {return Err("installation_id_change_refused".into());}
    if previous.as_ref().is_some_and(|i|i.manifest.installation_id!=input.installation_id) {return Err("installation_id_change_refused".into());}
    if !crate::mcp_launcher::core::manifest::is_id(&input.installation_id) {return Err("installation_id_invalid".into());}
    if !crate::mcp_launcher::core::manifest::is_sha256(&input.runtime_sha256) || !crate::mcp_launcher::core::manifest::is_sha256(&input.launcher_sha256) {return Err("generation_payload_hash_invalid".into());}
    let generation=match previous.as_ref().map(|p|p.manifest.generation).or_else(||history.as_ref().map(|h|h.generation)) {Some(generation)=>generation.checked_add(1).ok_or("runtime_generation_overflow")?,None=>1};
    let runtime_file=format!("mcp/runtimes/{}/{}",input.runtime_sha256,os::CLI_NAME);
    let mcp=root.join("mcp");
    ensure_directory(root,&mcp)?;
    let identity=mcp.join("installation.id");
    if identity.exists() {
        os::validate_tree(root,&identity)?;
        if std::fs::read_to_string(&identity).map_err(|e|e.to_string())?!=input.installation_id {return Err("installation_id_mismatch".into());}
    } else {create_once(&identity,input.installation_id.as_bytes())?;}
    for name in ["update-owner.lock","control.lock","operations.lock"] {
        let path=mcp.join(name);
        if !path.exists() {create_once(&path,b"")?;}
        os::validate_tree(root,&path)?;
    }
    let runtime_dir=mcp.join("runtimes").join(&input.runtime_sha256);
    ensure_directory(root,&runtime_dir)?;
    copy_immutable(root,&input.runtime_path,&root.join(&runtime_file),&input.runtime_sha256)?;
    let probe=crate::mcp_launcher::contract_probe::verify_runtime(root,&root.join(&runtime_file),&input.runtime_sha256,&input.product_version)?;
    if !runtime_dir.join("lifetime.lock").exists() {create_once(&runtime_dir.join("lifetime.lock"),b"")?;}
    let launcher_dir=mcp.join("launchers").join(&input.launcher_sha256);
    ensure_directory(root,&launcher_dir)?;
    copy_immutable(root,&input.launcher_path,&launcher_dir.join(os::LAUNCHER_NAME),&input.launcher_sha256)?;
    crate::mcp_launcher::contract_probe::verify_launcher(root,&launcher_dir.join(os::LAUNCHER_NAME),&input.launcher_sha256,&input.product_version)?;
    let info=serde_json::json!({"product_version":input.product_version,"launcher_protocol":1});
    let info_path=launcher_dir.join("info.v1.json");
    if info_path.exists() {
        let previous:serde_json::Value=crate::mcp_launcher::installation::read_json(&info_path)?;
        if previous!=info {return Err("launcher_build_metadata_mismatch".into());}
    } else {create_once(&info_path,&serde_json::to_vec(&info).map_err(|e|e.to_string())?)?;}
    os::validate_tree(root,&runtime_dir.join("lifetime.lock"))?;
    let compatibility=previous.as_ref().map(|old|crate::mcp_launcher::compatible_runtimes(old,&input.runtime_sha256,&probe.compatibility_catalog)).unwrap_or_default();
    let mut allowed=crate::mcp_launcher::compatible_launchers(root,&probe.compatibility_catalog);
    if !allowed.contains(&input.launcher_sha256) {allowed.push(input.launcher_sha256.clone());}
    let manifest=Manifest {schema:1,installation_id:input.installation_id.clone(),committed_transaction_id:unique_id()?,product_version:input.product_version.clone(),generation,runtime_sha256:input.runtime_sha256.clone(),runtime_file,launcher_protocol:probe.launcher_protocol,launcher_sha256:input.launcher_sha256.clone(),launcher_product_version:input.product_version.clone(),allowed_launcher_hashes:allowed,ipc_contract_id:probe.ipc_contract_id,shared_state_contract_id:probe.shared_state_contract_id,supported_runtimes:compatibility.supported,compatibility_catalog_sha256:probe.compatibility_catalog_sha256};
    manifest.validate(os::CLI_NAME)?;Ok(manifest)
}
pub fn install_generation(root:&Path,input:&GenerationInput)->Result<Manifest,String> {
    if root.join("mcp/current.v1.json").exists() {return Err("existing_installation_requires_update_transaction".into());}
    let manifest=stage_generation(root,input)?;
    commit_initial_generation(root,&manifest)?;
    Installation::load(root)?;
    Ok(manifest)
}
fn ensure_directory(root:&Path,path:&Path)->Result<(),String> {
    os::validate_tree(root,root)?;
    // Create one checked component at a time; never chmod through a link.
    let mut current=root.to_path_buf();
    for component in path.strip_prefix(root).map_err(|e|e.to_string())?.components() {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(_) => os::validate_tree(root,&current)?,
            Err(e) if e.kind()==std::io::ErrorKind::NotFound => {os::create_directory(&current)?;},
            Err(e)=>return Err(e.to_string()),
        }
    }
    os::validate_tree(root,path)
}
fn create_once(path:&Path,bytes:&[u8])->Result<(),String> {
    let mut file=tempfile::NamedTempFile::new_in(path.parent().ok_or("runtime_record_parent_missing")?).map_err(|e|e.to_string())?;
    file.write_all(bytes).and_then(|()|file.as_file().sync_all()).map_err(|e|e.to_string())?;
    os::protect(file.path(),false)?;os::publish_staged(file,path,false)
}
fn copy_immutable(root:&Path,source:&Path,target:&Path,hash:&str)->Result<(),String> {
    if target.exists() {let _checked=os::open_verified(root,target,hash)?;return Ok(());}
    let mut source=File::open(source).map_err(|e|e.to_string())?;
    verify_handle(&mut source,hash)?;
    let parent=target.parent().ok_or("generation_parent_missing")?;
    let mut staged=tempfile::NamedTempFile::new_in(parent).map_err(|e|e.to_string())?;
    std::io::copy(&mut source,&mut staged).map_err(|e|e.to_string())?;
    staged.as_file().sync_all().map_err(|e|e.to_string())?;
    os::protect(staged.path(),true)?;
    verify_handle(staged.as_file_mut(),hash)?;
    os::publish_staged(staged,target,false)?;
    let _checked=os::open_verified(root,target,hash)?;
    Ok(())
}
pub(crate) fn unique_id()->Result<String,String> {
    let mut bytes=[0u8;16];getrandom::getrandom(&mut bytes).map_err(|e|e.to_string())?;
    Ok(bytes.iter().map(|b|format!("{b:02x}")).collect())
}

/// Held across the ordinary file replacement on first managed installation.
/// Acquiring only at pointer publication would permit two physical installers.
pub struct InitialGenerationLease { installation: Installation, _owner: File }
impl InitialGenerationLease {
    pub fn prepare(root:&Path,manifest:&Manifest)->Result<Self,String> {
        manifest.validate(os::CLI_NAME)?;
        crate::mcp_launcher::contract_probe::verify_manifest_policy(root,manifest)?;
        let installation=Installation {root:std::fs::canonicalize(root).map_err(|e|e.to_string())?,manifest:manifest.clone()};
        let owner=installation.lock("update-owner.lock",true)?;
        let _control=installation.lock("control.lock",true)?;
        if root.join("mcp/uninstall.v1.json").exists() {return Err("installation_uninstall_in_progress".into());}
        if installation.root.join("mcp/current.v1.json").exists() {return Err("initial_generation_already_committed".into());}
        Ok(Self {installation,_owner:owner})
    }
    /// Publish only after the regular installed CLI and immutable copy agree.
    pub fn commit(&self)->Result<(),String> {
        let root=&self.installation.root;let manifest=&self.installation.manifest;
        let _control=self.installation.lock("control.lock",true)?;
        if root.join("mcp/current.v1.json").exists() {return Err("initial_generation_already_committed".into());}
        let _installed=os::open_verified(root,&root.join(os::CLI_NAME),&manifest.runtime_sha256)?;
        let _runtime=os::open_verified(root,&root.join(&manifest.runtime_file),&manifest.runtime_sha256)?;
        let pointer=root.join("mcp/current.v1.json");
        if let Err(error)=atomic_json(&pointer,manifest) {
            if pointer.exists() {
                let actual:Manifest=crate::mcp_launcher::installation::read_json(&pointer)?;
                if actual!=*manifest {return Err(format!("{error}; initial_generation_recovery_required: pointer changed"));}
                retire_current_pointer(root).map_err(|rollback|format!("{error}; initial_generation_recovery_required: {rollback}"))?;
            }
            return Err(error);
        }
        Ok(())
    }
}
pub fn commit_initial_generation(root:&Path,manifest:&Manifest)->Result<(),String> {
    InitialGenerationLease::prepare(root,manifest)?.commit()
}
pub fn new_installation_id()->Result<String,String> {unique_id()}
/// A successful legacy downgrade cannot advertise a newer private generation.
pub fn disable_legacy_generation(root:&Path)->Result<(),String> {
    if root.join("mcp/current.v1.json").exists() {return Err("existing_managed_installation_requires_retirement_transaction".into());}
    Ok(())
}

pub fn validate_installation_root(root:&Path)->Result<(),String> {os::validate_tree(root,root)}
pub fn validate_installation_path(root:&Path,path:&Path)->Result<(),String> {os::validate_tree(root,path)}

/// The same hash-bound contract probe confirms installation completion
/// without starting a GUI, daemon or user MCP server in an admin context.
pub fn verify_installed_product(root:&Path,path:&Path,hash:&str,version:&str)->Result<(),String> {
    crate::mcp_launcher::contract_probe::verify_runtime(root,path,hash,version).map(|_|())
}

pub(crate) fn retire_current_pointer(root:&Path)->Result<(),String> {
    let retired=root.join("mcp").join(format!("retired-current-{}.json",unique_id()?));
    os::rename_durable(&root.join("mcp/current.v1.json"),&retired)
}

/// Probe in isolated staging; no installation generation, marker or lock is
/// changed. Hash-bound native execution is shared with normal provisioning.
pub fn staged_runtime_managed(path:&Path,hash:&str,version:&str)->Result<bool,String> {
    let directory=tempfile::Builder::new().prefix("se-mcp-capability-").tempdir().map_err(|e|e.to_string())?;
    let root=std::fs::canonicalize(directory.path()).map_err(|e|e.to_string())?;os::protect(&root,true)?;
    let target=root.join(os::CLI_NAME);copy_immutable(&root,path,&target,hash)?;
    crate::mcp_launcher::contract_probe::classify_runtime(&root,&target,hash,version)
}
pub fn verify_staged_launcher(path:&Path,hash:&str,version:&str)->Result<(),String> {
    let directory=tempfile::Builder::new().prefix("se-launcher-capability-").tempdir().map_err(|e|e.to_string())?;
    let root=std::fs::canonicalize(directory.path()).map_err(|e|e.to_string())?;os::protect(&root,true)?;
    let target=root.join(os::LAUNCHER_NAME);copy_immutable(&root,path,&target,hash)?;
    crate::mcp_launcher::contract_probe::verify_launcher(&root,&target,hash,version)
}
