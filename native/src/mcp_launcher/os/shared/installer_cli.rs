//! Explicit installer mode, separate from the root-derived MCP launcher path.
use crate::mcp_launcher::{GenerationInput,Installation};
use std::ffi::OsString;
use std::path::PathBuf;

#[cfg(all(test, any(windows, target_os = "linux")))]
#[path = "batches_uninstall_task_tests.rs"]
mod batches_uninstall_task_tests;

pub(crate) fn provision(arguments:&[OsString])->Result<i32,String> {
    let allowed=["--root","--runtime-source","--runtime-sha256","--launcher-source","--launcher-sha256","--version"];
    let mut values=std::collections::BTreeMap::new();
    for pair in arguments.chunks(2) {
        if pair.len()!=2 {return Err("provision argument value missing".into());}
        let key=pair[0].to_str().ok_or("provision argument invalid")?;
        if !allowed.contains(&key) || values.insert(key,pair[1].clone()).is_some() {return Err(format!("provision unknown or duplicate argument: {key}"));}
    }
    let get=|key:&str|values.get(key).cloned().ok_or_else(||format!("provision required argument missing: {key}"));
    let root=std::fs::canonicalize(PathBuf::from(get("--root")?)).map_err(|e|e.to_string())?;
    if root.join("mcp/uninstall.v1.json").exists() {return Err("installation_uninstall_in_progress".into());}
    let installation_id=match std::fs::read_to_string(root.join("mcp/installation.id")) {
        Ok(id)=>id,Err(e) if e.kind()==std::io::ErrorKind::NotFound=>crate::mcp_launcher::new_installation_id()?,Err(e)=>return Err(e.to_string()),
    };
    let text=|key:&str|get(key)?.into_string().map_err(|_|format!("provision argument must be UTF-8: {key}"));
    let input=GenerationInput {installation_id,product_version:text("--version")?,runtime_path:PathBuf::from(get("--runtime-source")?),runtime_sha256:text("--runtime-sha256")?,launcher_path:PathBuf::from(get("--launcher-source")?),launcher_sha256:text("--launcher-sha256")?};
    if root.join("mcp/current.v1.json").exists() {
        let installed=Installation::load(&root)?;
        if installed.manifest.runtime_sha256==input.runtime_sha256 && installed.manifest.launcher_sha256==input.launcher_sha256 && installed.manifest.product_version==input.product_version {
            installed.checked_launcher_path()?;
            crate::mcp_launcher::verify_installed_product(&root,&installed.installed_cli(),&input.runtime_sha256,&input.product_version)?;
            crate::mcp_launcher::contract_probe::verify_runtime(&root,&installed.runtime_path(),&input.runtime_sha256,&input.product_version)?;
            return Ok(0);
        }
        return Err("existing installation requires the verified updater apply transaction".into());
    }
    let manifest=crate::mcp_launcher::stage_generation(&root,&input)?;
    crate::mcp_launcher::commit_initial_generation(&root,&manifest)?;
    Ok(0)
}

use crate::mcp_launcher::{os,Manifest};
use crate::mcp_launcher::installation::{hash_bytes,read_json,verify_handle};
use std::path::Path;
use serde::{Deserialize,Serialize};

#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct RemovalFile {path:String,sha256:String}
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct RemovalRecord {schema:u32,manifest:Manifest,files:Vec<RemovalFile>,directories:Vec<String>}

/// The installer runs the installed CLI, never a runtime or profile-selected
/// path. Only this installation's program files are in the removal boundary.
pub(crate) fn retire_current_installation()->Result<(),String> {
    if crate::mcp_launcher::current_runtime().is_some() {return Err("uninstall_requires_installed_cli".into());}
    let image=std::fs::canonicalize(std::env::current_exe().map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    let root=image.parent().ok_or("installation_root_missing")?;
    if image!=root.join(os::CLI_NAME) {return Err("uninstall_requires_installed_cli".into());}
    os::uninstall_programs_idle(root)?;
    if !exists(&root.join("mcp"))? {return Ok(());}
    let record=load_removal(root)?;
    if record.is_none() && !exists(&root.join("mcp/current.v1.json"))? {
        if remaining_managed_files(root)? {return Err("uninstall_mcp_manifest_missing: recover this installation before removal".into());}
        return Ok(());
    }
    let manifest=match &record {Some(record)=>record.manifest.clone(),None=>Installation::load(root)?.manifest};
    let mut running=os::running_image()?;
    verify_handle(&mut running,&manifest.runtime_sha256)?;
    drop(running);
    retire_mcp(root,record)
}

fn load_removal(root:&Path)->Result<Option<RemovalRecord>,String> {
    let path=root.join("mcp/uninstall.v1.json");
    if !exists(&path)? {return Ok(None);}
    os::validate_tree(root,&path)?;
    let record:RemovalRecord=read_json(&path)?;
    record.manifest.validate(os::CLI_NAME)?;
    let mut names=std::collections::BTreeSet::new();
    if record.schema!=1 || record.files.iter().any(|file|!removal_file_allowed(&file.path)
        || !crate::mcp_launcher::core::manifest::is_sha256(&file.sha256) || !names.insert(file.path.clone()))
        || record.directories.iter().any(|path|!removal_directory_allowed(path))
        || ["mcp/current.v1.json","mcp/installation.id","mcp/update-owner.lock","mcp/control.lock","mcp/operations.lock"].iter().any(|name|!names.contains(*name)) {
        return Err("uninstall_record_invalid".into());
    }
    Ok(Some(record))
}

fn retire_mcp(root:&Path,record:Option<RemovalRecord>)->Result<(),String> {
    let manifest=match &record {Some(record)=>record.manifest.clone(),None=>Installation::load(root)?.manifest};
    let installation=Installation {root:root.into(),manifest};
    // Missing locks are accepted only during a recorded removal, after every
    // executable was already removed. Never recreate a lock while it is used.
    let lock=|name:&str| {
        if !exists(&root.join("mcp").join(name))? && record.is_some() {return Ok(None);}
        installation.lock(name,true).map(Some)
    };
    let owner=lock("update-owner.lock")?;
    let control=lock("control.lock")?;
    let operations=lock("operations.lock")?;
    if record.is_none() && Installation::load(root)?.manifest!=installation.manifest {return Err("uninstall_installation_changed".into());}
    if installation.state()?.is_some_and(|state|state.blocks_work() || (state.retire_managed && state.phase==crate::mcp_launcher::core::lifecycle::Phase::Committed && installation.manifest==state.previous)) {return Err("uninstall_requires_update_recovery".into());}
    let record=match record {Some(record)=>record,None=>inventory(&installation)?};
    if owner.is_none() || control.is_none() || operations.is_none() {
        for file in &record.files {
            if (file.path.ends_with(&format!("/{}",os::CLI_NAME)) || file.path.ends_with(&format!("/{}",os::LAUNCHER_NAME))) && exists(&root.join(&file.path))? {
                return Err("uninstall_missing_locks_with_executable_owners_unproved".into());
            }
        }
    }
    let mut lifetimes=Vec::new();
    for file in &record.files {
        let path=root.join(&file.path);
        if !exists(&path)? {continue;}
        verify_removal_file(root,file)?;
        if file.path.ends_with(&format!("/{}",os::CLI_NAME)) || file.path.ends_with(&format!("/{}",os::LAUNCHER_NAME)) {
            if !os::processes_for(&path)?.is_empty() {return Err(format!("uninstall_busy: {} still has program owners",path.display()));}
            if file.path.starts_with("mcp/runtimes/") {
                let lifetime=path.parent().ok_or("runtime_parent_missing")?.join("lifetime.lock");
                os::validate_tree(root,&lifetime)?;
                lifetimes.push(os::lock(&lifetime,true)?);
            }
        }
    }
    // This durable, validated inventory also blocks every new managed start.
    // A partial deletion is retried against these exact original hashes.
    write_removal_record(root,&record)?;
    for file in record.files.iter().filter(|file|!file.path.ends_with(".lock")) {remove_proved_file(root,file)?;}
    drop(lifetimes);drop(operations);drop(control);drop(owner);
    for file in record.files.iter().filter(|file|file.path.ends_with(".lock")) {remove_proved_file(root,file)?;}
    for directory in &record.directories {remove_empty(root,&root.join(directory))?;}
    std::fs::remove_file(root.join("mcp/uninstall.v1.json")).map_err(|e|e.to_string())?;
    remove_empty(root,&root.join("mcp"))
}

fn write_removal_record(root:&Path,record:&RemovalRecord)->Result<(),String> {
    // Match read_json's existing budget before exposing an unrecoverable record.
    let bytes=serde_json::to_vec(record).map_err(|e|e.to_string())?;
    if bytes.len()>256*1024 {return Err("uninstall_record_exceeds_record_budget: owned files retained".into());}
    crate::mcp_launcher::installation::atomic_bytes(&root.join("mcp/uninstall.v1.json"),&bytes)
}

fn inventory(installation:&Installation)->Result<RemovalRecord,String> {
    let root=&installation.root;let id=&installation.manifest.installation_id;
    let mut files=Vec::new();let mut directories=Vec::new();
    for category in ["runtimes","launchers"] {
        let directory=root.join("mcp").join(category);
        if !exists(&directory)? {continue;}
        os::validate_tree(root,&directory)?;
        for entry in std::fs::read_dir(&directory).map_err(|e|e.to_string())? {
            let entry=entry.map_err(|e|e.to_string())?;
            let Some(name)=entry.file_name().to_str().map(str::to_owned) else {continue;};
            let Some(hash)=generation_hash(category,&name) else {continue;};
            let relative=format!("mcp/{category}/{name}");
            os::validate_tree(root,&entry.path())?;
            let executable=if category=="runtimes" {os::CLI_NAME}else{os::LAUNCHER_NAME};
            let image=format!("{relative}/{executable}");
            if exists(&root.join(&image))? {
                let checked=os::open_verified(root,&root.join(&image),&hash)?;drop(checked);
                files.push(RemovalFile {path:image,sha256:hash});
            }
            let metadata=if category=="runtimes" {"lifetime.lock"}else{"info.v1.json"};
            let metadata=format!("{relative}/{metadata}");
            if exists(&root.join(&metadata))? {
                os::validate_tree(root,&root.join(&metadata))?;
                let bytes=std::fs::read(root.join(&metadata)).map_err(|e|e.to_string())?;
                if category=="runtimes" {
                    if !bytes.is_empty() {return Err("uninstall_runtime_lock_content_changed".into());}
                } else {
                    let value:serde_json::Value=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;
                    if value["product_version"].as_str().is_none_or(str::is_empty)
                        || value!=serde_json::json!({"product_version":value["product_version"],"launcher_protocol":1}) {
                        return Err("uninstall_launcher_metadata_changed".into());
                    }
                }
                files.push(RemovalFile {path:metadata,sha256:hash_bytes(&bytes)});
            }
            directories.push(relative);
        }
        directories.push(format!("mcp/{category}"));
    }
    for entry in std::fs::read_dir(root.join("mcp")).map_err(|e|e.to_string())? {
        let entry=entry.map_err(|e|e.to_string())?;
        let Some(name)=entry.file_name().to_str().map(str::to_owned) else {continue;};
        let relative=format!("mcp/{name}");
        if !removal_top_file(&name) {continue;}
        os::validate_tree(root,&entry.path())?;
        let bytes=std::fs::read(entry.path()).map_err(|e|e.to_string())?;
        if name=="installation.id" {
            if bytes!=id.as_bytes() {return Err("uninstall_installation_id_changed".into());}
        } else if name=="install-last-applied.txt" {
            // NSIS's verified bundle helper writes only the bare release
            // version here. Preserve a user-repurposed file at this name.
            let version=std::str::from_utf8(&bytes).ok();
            if !version.is_some_and(|version|version.len()<=64 && version.split('.').count()==3
                && version.split('.').all(|part|!part.is_empty() && part.bytes().all(|byte|byte.is_ascii_digit()))) {continue;}
        } else if name.ends_with(".lock") {
            if !bytes.is_empty() {return Err("uninstall_lock_content_changed".into());}
        } else if name=="update.stop" {
            if bytes!=b"stop" {return Err("uninstall_stop_content_changed".into());}
        } else if name=="update.v1.json" {
            let state:crate::mcp_launcher::core::lifecycle::UpdateState=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;
            state.validate(os::CLI_NAME)?;
            if &state.installation_id!=id || state.blocks_work() {return Err("uninstall_requires_update_recovery".into());}
        } else {
            let manifest:Manifest=serde_json::from_slice(&bytes).map_err(|e|e.to_string())?;manifest.validate(os::CLI_NAME)?;
            if &manifest.installation_id!=id {return Err("uninstall_foreign_manifest".into());}
        }
        files.push(RemovalFile {path:relative,sha256:hash_bytes(&bytes)});
    }
    Ok(RemovalRecord {schema:1,manifest:installation.manifest.clone(),files,directories})
}
fn verify_removal_file(root:&Path,file:&RemovalFile)->Result<(),String> {
    os::validate_tree(root,&root.join(&file.path))?;
    if crate::updater::file_sha256(&root.join(&file.path))?!=file.sha256 {return Err(format!("uninstall_content_changed: {}",file.path));}
    Ok(())
}
fn remove_proved_file(root:&Path,file:&RemovalFile)->Result<(),String> {
    if !exists(&root.join(&file.path))? {return Ok(());}
    verify_removal_file(root,file)?;
    std::fs::remove_file(root.join(&file.path)).map_err(|e|format!("{}: {e}",file.path))
}
fn exists(path:&Path)->Result<bool,String> {
    match std::fs::symlink_metadata(path) {Ok(_)=>Ok(true),Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(false),Err(e)=>Err(e.to_string())}
}
fn remove_empty(root:&Path,path:&Path)->Result<(),String> {
    if !exists(path)? {return Ok(());}
    os::validate_tree(root,path)?;
    match std::fs::remove_dir(path) {
        Ok(())=>Ok(()),Err(e) if e.kind()==std::io::ErrorKind::DirectoryNotEmpty=>Ok(()),Err(e)=>Err(e.to_string()),
    }
}
fn generation_hash(category:&str,name:&str)->Option<String> {
    use crate::mcp_launcher::core::manifest::{is_id,is_sha256};
    if is_sha256(name) {return Some(name.into());}
    if category=="runtimes" {
        let (hash,id)=name.strip_prefix(".retired-")?.split_once('-')?;
        if is_sha256(hash) && is_id(id) {return Some(hash.into());}
    }
    None
}
fn remaining_managed_files(root:&Path)->Result<bool,String> {
    let mcp=root.join("mcp");os::validate_tree(root,&mcp)?;
    for entry in std::fs::read_dir(&mcp).map_err(|e|e.to_string())? {
        let entry=entry.map_err(|e|e.to_string())?;
        if entry.file_name().to_str().is_some_and(|name|name!="install-last-applied.txt" && removal_top_file(name)) {return Ok(true);}
    }
    for category in ["runtimes","launchers"] {
        let directory=mcp.join(category);if !exists(&directory)? {continue;}
        os::validate_tree(root,&directory)?;
        for entry in std::fs::read_dir(directory).map_err(|e|e.to_string())? {
            let entry=entry.map_err(|e|e.to_string())?;
            if !entry.file_name().to_str().is_some_and(|name|generation_hash(category,name).is_some()) {continue;}
            os::validate_tree(root,&entry.path())?;
            for name in if category=="runtimes" {[os::CLI_NAME,"lifetime.lock"]}else{[os::LAUNCHER_NAME,"info.v1.json"]} {
                if exists(&entry.path().join(name))? {return Ok(true);}
            }
        }
    }
    Ok(false)
}
fn removal_top_file(name:&str)->bool {
    matches!(name,"current.v1.json"|"legacy.v1.json"|"installation.id"|"install-last-applied.txt"|"update.v1.json"|"update.stop"|"update-owner.lock"|"control.lock"|"operations.lock")
        || name.strip_prefix("retired-current-").and_then(|name|name.strip_suffix(".json"))
            .is_some_and(crate::mcp_launcher::core::manifest::is_id)
}
fn removal_directory_allowed(path:&str)->bool {
    if matches!(path,"mcp/runtimes"|"mcp/launchers") {return true;}
    let parts=path.split('/').collect::<Vec<_>>();
    parts.len()==3 && parts[0]=="mcp" && matches!(parts[1],"runtimes"|"launchers")
        && generation_hash(parts[1],parts[2]).is_some()
}
fn removal_file_allowed(path:&str)->bool {
    if let Some(name)=path.strip_prefix("mcp/") {if removal_top_file(name) {return true;}}
    let parts=path.split('/').collect::<Vec<_>>();
    parts.len()==4 && removal_directory_allowed(&parts[..3].join("/"))
        && if parts[1]=="runtimes" {matches!(parts[3],"lifetime.lock") || parts[3]==os::CLI_NAME}
           else {parts[3]=="info.v1.json" || parts[3]==os::LAUNCHER_NAME}
}
