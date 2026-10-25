//! The update owner never waits for operations while holding control.lock.
use crate::mcp_launcher::{os,Installation,Manifest};
use crate::mcp_launcher::core::lifecycle::{Phase,UpdateState,InstallationTarget};
use crate::mcp_launcher::installation::{atomic_bytes,atomic_json,hash_bytes};
use std::fs::File;
use std::path::Path;
use std::time::{Duration,Instant};

pub struct UpdateTransaction {
    installation:Installation,
    state:UpdateState,
    _owner:File,
    operations:Option<File>,
    completed:bool,
}
impl UpdateTransaction {
    pub fn prepare(root:&Path,target:Manifest)->Result<Self,String> {
        let previous=Installation::load(root)?;
        let proof=InstallationTarget {file:os::CLI_NAME.into(),previous_sha256:Some(previous.manifest.runtime_sha256),target_sha256:target.runtime_sha256.clone()};
        Self::prepare_with_targets(root,target,vec![proof])
    }
    pub fn prepare_with_targets(root:&Path,target:Manifest,installation_targets:Vec<InstallationTarget>)->Result<Self,String> {
        Self::prepare_internal(root,target,installation_targets,false)
    }
    pub fn prepare_retirement(root:&Path,version:&str,installation_targets:Vec<InstallationTarget>)->Result<Self,String> {
        let previous=Installation::load(root)?;
        let mut target=previous.manifest.clone();
        target.product_version=version.into();target.generation=target.generation.checked_add(1).ok_or("runtime_generation_overflow")?;
        target.committed_transaction_id=crate::mcp_launcher::provision::unique_id()?;
        target.supported_runtimes.clear();
        Self::prepare_internal(root,target,installation_targets,true)
    }
    fn prepare_internal(root:&Path,target:Manifest,installation_targets:Vec<InstallationTarget>,retire_managed:bool)->Result<Self,String> {
        let installation=Installation::load(root)?;
        target.validate(os::CLI_NAME)?;
        let owner=installation.lock("update-owner.lock",true)?;
        if !retire_managed {crate::mcp_launcher::contract_probe::verify_manifest_policy(root,&target)?;}
        let _control=installation.lock("control.lock",true)?;
        let installation=Installation::load(root)?;
        if target.installation_id!=installation.manifest.installation_id
            || target.generation<=installation.manifest.generation {return Err("update_target_identity_or_generation_invalid".into());}
        if installation.state()?.is_some_and(|state|state.blocks_work() || (state.retire_managed && state.phase==Phase::Committed && installation.manifest==state.previous)) {return Err("update_recovery_required".into());}
        let _image=os::open_verified(root,&root.join(&target.runtime_file),&target.runtime_sha256)?;
        let launcher=root.join("mcp/launchers").join(&target.launcher_sha256).join(os::LAUNCHER_NAME);
        let _launcher=os::open_verified(root,&launcher,&target.launcher_sha256)?;
        let state=UpdateState {schema:1,installation_id:target.installation_id.clone(),transaction_id:target.committed_transaction_id.clone(),target_manifest_sha256:hash_bytes(&serde_json::to_vec(&target).map_err(|e|e.to_string())?),owner:os::process_identity(std::process::id())?,phase:Phase::Prepared,target,previous:installation.manifest.clone(),stop:None,legacy_stop:None,installation_targets,retire_managed};
        state.validate(os::CLI_NAME)?;
        atomic_json(&root.join("mcp/update.v1.json"),&state)?;
        drop(_control);
        Ok(Self{installation,state,_owner:owner,operations:None,completed:false})
    }
    pub fn drain(&mut self,timeout:Duration)->Result<(),String> {
        let deadline=Instant::now()+timeout.min(Duration::from_secs(30));
        loop {
            match self.installation.lock("operations.lock",true) {
                Ok(lock)=>{self.operations=Some(lock);return Ok(());},
                Err(error) if error.starts_with("lock_busy:") && Instant::now()<deadline=>std::thread::sleep(Duration::from_millis(50)),
                Err(error)=>{
                    self.rollback()?;
                    return Err(format!("{}: {error}",if error.starts_with("lock_busy:") {"update_deferred_busy"} else {"update_operations_lock_failed"}));
                }
            }
        }
    }
    pub fn publish_stop(&mut self,marker:&Path)->Result<(),String> {
        if self.operations.is_none() {return Err("update_stop_requires_drain".into());}
        let _control=self.installation.lock("control.lock",true)?;
        // Exact legacy payload, published with the identity-bound control record.
        let parent=marker.parent().ok_or("stop_parent_missing")?;
        std::fs::create_dir_all(parent).map_err(|e|e.to_string())?;
        let global=self.installation.root.join("mcp/update.stop");
        atomic_bytes(&global,b"stop")?;
        atomic_bytes(marker,b"stop")?;
        self.state.stop=Some(os::stop_identity(&global)?);
        self.state.legacy_stop=Some(os::stop_identity(marker)?);
        self.state.phase=Phase::Stopping;
        atomic_json(&self.installation.root.join("mcp/update.v1.json"),&self.state)
    }
    /// Called only after regular replacement and acknowledged new app startup.
    pub fn commit(&mut self)->Result<(),String> {
        let _control=self.installation.lock("control.lock",true)?;
        if self.state.retire_managed {
            verify_target_proofs(&self.installation.root,&self.state,true)?;
            atomic_json(&self.installation.root.join("mcp/legacy.v1.json"),&self.state.previous)?;
            self.state.phase=Phase::Committed;
            atomic_json(&self.installation.root.join("mcp/update.v1.json"),&self.state)?;
            let pointer=self.installation.root.join("mcp/current.v1.json");
            let retired=crate::mcp_launcher::provision::retire_current_pointer(&self.installation.root);
            if let Err(error)=retired {
                if !pointer.exists() {atomic_json(&pointer,&self.state.previous).map_err(|restore|format!("{error}; retirement_recovery_required: {restore}"))?;}
                return Err(error);
            }
            self.completed=true;return Ok(());
        }
        let installed=self.installation.installed_cli();
        let _installed=os::open_verified(&self.installation.root,&installed,&self.state.target.runtime_sha256)?;
        verify_target_proofs(&self.installation.root,&self.state,true)?;
        let _image=os::open_verified(&self.installation.root,&self.installation.root.join(&self.state.target.runtime_file),&self.state.target.runtime_sha256)?;
        atomic_json(&self.installation.root.join("mcp/current.v1.json"),&self.state.target)?;
        self.state.phase=Phase::Committed;
        if let Err(error)=atomic_json(&self.installation.root.join("mcp/update.v1.json"),&self.state) {
            atomic_json(&self.installation.root.join("mcp/current.v1.json"),&self.state.previous).map_err(|rollback|format!("{error}; update_recovery_required: pointer rollback: {rollback}"))?;
            self.state.phase=Phase::Stopping;
            return Err(error);
        }
        self.completed=true;
        Ok(())
    }
    pub fn rollback(&mut self)->Result<(),String> {
        let _control=self.installation.lock("control.lock",true)?;
        let active=Installation::load(&self.installation.root)?;
        if active.manifest!=self.state.previous {return Err("update_rollback_manifest_changed_recovery_required".into());}
        verify_target_proofs(&self.installation.root,&self.state,false)?;
        self.state.phase=Phase::RolledBack;
        atomic_json(&self.installation.root.join("mcp/update.v1.json"),&self.state)?;
        self.completed=true;self.operations.take();Ok(())
    }
}
impl Drop for UpdateTransaction {
    fn drop(&mut self) {
        // No optimistic rollback on error/crash: regular targets might already
        // have changed. The persistent prepared record keeps work blocked.
        if !self.completed {eprintln!("se updater: update_recovery_required transaction {}",self.state.transaction_id);}
    }
}

/// Explicit recovery checks the owner lock, process identity, confirmed
/// pointer and installed hash together. No deletion based on a missing PID.
pub fn recover_update(root:&Path,complete:bool)->Result<(),String> {
    let installation=Installation::load(root)?;
    let _owner=installation.lock("update-owner.lock",true)?;
    let _control=installation.lock("control.lock",true)?;
    let Some(mut state)=installation.state()? else {return Ok(());};
    if !state.blocks_work() && !(state.retire_managed && state.phase==Phase::Committed && installation.manifest==state.previous) {return Ok(());}
    if os::process_matches(&state.owner) {
        return Err("update_owner_still_alive".into());
    }
    verify_target_proofs(root,&state,complete)?;
    if state.retire_managed && complete {
        atomic_json(&root.join("mcp/legacy.v1.json"),&state.previous)?;
        crate::mcp_launcher::provision::retire_current_pointer(root)?;
        state.phase=Phase::Committed;return atomic_json(&root.join("mcp/update.v1.json"),&state);
    }
    let chosen=if complete {&state.target} else {&state.previous};
    let _installed=os::open_verified(root,&installation.installed_cli(),&chosen.runtime_sha256)?;
    let _runtime=os::open_verified(root,&root.join(&chosen.runtime_file),&chosen.runtime_sha256)?;
    // The caller additionally proves regular app/updater rollback or startup;
    // this API does not reconstruct or copy partial release targets.
    atomic_json(&root.join("mcp/current.v1.json"),chosen)?;
    state.phase=if complete {Phase::Committed}else{Phase::RolledBack};
    atomic_json(&root.join("mcp/update.v1.json"),&state)
}

pub fn retain_update_stop(marker:&Path)->Result<bool,String> {
    Ok(retained_update_transaction(marker)?.is_some())
}
pub fn retained_update_transaction(marker:&Path)->Result<Option<String>,String> {
    if crate::mcp_launcher::migration_entry() {return Ok(None);}

    let Some(runtime)=crate::mcp_launcher::current_runtime() else {return Ok(None);};
    let installation=Installation::load(&runtime.installation.root)?;
    let control=installation.lock("control.lock",false)?;
    let Some(state)=installation.state()? else {return Ok(None);};
    let expected=if marker==installation.root.join("mcp/update.stop") {state.stop.as_ref()} else {state.legacy_stop.as_ref()};
    if expected!=Some(&os::stop_identity(marker)?) {return Ok(None);}
    if state.retains(&installation.manifest.installation_id,&runtime.contract)
        && installation.owner_alive(&state) {return Ok(Some(state.transaction_id));}
    // A fast completed update can finish before the 1 s stop poll. Consume
    // only that exact already-processed marker, with the confirmed pointer
    // and a new daemon Hello. Prepared/crashed owners never enter this path.
    let completed=match state.phase {
        Phase::Committed=>!state.retire_managed && installation.manifest==state.target,
        Phase::RolledBack=>installation.manifest==state.previous,
        _=>false,
    };
    if completed && installation.manifest.permits(&runtime.contract) {
        drop(control);
        crate::daemon::runtime_hello(&runtime)?;
        return Ok(Some(state.transaction_id));
    }
    Ok(None)
}
pub fn stop_marker_is_owned(root:&Path,marker:&Path)->Result<bool,String> {
    let installation=Installation::load(root)?;
    let _control=installation.lock("control.lock",false)?;
    Ok(installation.state()?.is_some_and(|state|state.blocks_work()
        && installation.owner_alive(&state) && state.legacy_stop.as_ref()==os::stop_identity(marker).ok().as_ref()))
}

pub fn garbage_collect(root:&Path)->Result<Vec<String>,String> {
    let installation=Installation::load(root)?;
    let _owner=installation.lock("update-owner.lock",true)?;
    let _control=installation.lock("control.lock",true)?;
    let state=installation.state()?;
    if state.as_ref().is_some_and(|s|s.blocks_work()) {return Err("update_recovery_required".into());}
    let mut removed=Vec::new();
    for entry in std::fs::read_dir(root.join("mcp/runtimes")).map_err(|e|e.to_string())? {
        let entry=entry.map_err(|e|e.to_string())?;
        let Some(hash)=entry.file_name().to_str().map(str::to_owned) else {continue;};
        if !crate::mcp_launcher::core::manifest::is_sha256(&hash)
            || hash==installation.manifest.runtime_sha256
            || state.as_ref().is_some_and(|s|s.previous.runtime_sha256==hash)
        {continue;}
        let path=entry.path();os::validate_tree(root,&path)?;
        let Ok(_lifetime)=os::lock(&path.join("lifetime.lock"),true) else {continue;};
        match os::processes_for(&path.join(os::CLI_NAME)) {Ok(processes) if processes.is_empty()=>{},_=>continue}
        // Quarantine by atomic rename while holding the exclusive lifetime
        // and control locks. A new opener can no longer resolve this hashpath.
        let retired=root.join("mcp/runtimes").join(format!(".retired-{hash}-{}",crate::mcp_launcher::provision::unique_id()?));
        // Windows read-only lock handles intentionally disallow deletion; a
        // failed quarantine preserves the generation for the next attempt.
        if os::rename_durable(&path,&retired).is_err() {continue;}
        drop(_lifetime);
        std::fs::remove_dir_all(&retired).map_err(|e|e.to_string())?;
        removed.push(hash);
    }
    Ok(removed)
}

pub fn stop_marker_identity(marker:&Path)->Option<crate::mcp_launcher::core::lifecycle::StopIdentity> {
    let identity=os::stop_identity(marker).ok()?;
    (identity.sha256==hash_bytes(b"stop")).then_some(identity)
}

pub fn installation_stop_marker()->Option<std::path::PathBuf> {
    Installation::current().ok().flatten().map(|i|i.root.join("mcp/update.stop"))
}

pub fn verify_recovery_targets(root:&Path,complete:bool)->Result<(),String> {
    let installation=Installation::load(root)?;
    let _owner=installation.lock("update-owner.lock",true)?;
    let _control=installation.lock("control.lock",true)?;
    let state=installation.state()?.ok_or("no pending MCP update")?;
    if !state.blocks_work() && !(state.retire_managed && state.phase==Phase::Committed && installation.manifest==state.previous) {return Err("no pending MCP update".into());}
    if os::process_matches(&state.owner) {return Err("update_owner_still_alive".into());}
    verify_target_proofs(root,&state,complete)
}
fn verify_target_proofs(root:&Path,state:&UpdateState,complete:bool)->Result<(),String> {
    if state.installation_targets.is_empty() {return Err("update_recovery_required: regular target proofs missing".into());}
    for target in &state.installation_targets {
        let path=root.join(&target.file);
        let hash=if complete {Some(target.target_sha256.as_str())}else{target.previous_sha256.as_deref()};
        match hash {Some(hash)=>{let _target=os::open_verified(root,&path,hash)?;},None if !path.exists()=>{},None=>return Err("update_recovery_required: previously absent target still exists".into())}
    }
    Ok(())
}
