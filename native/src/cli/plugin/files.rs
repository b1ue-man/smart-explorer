//! Owned profile files use recorded fingerprints and the installation's launcher.
#[cfg(test)]
#[path = "batches_legacy_lifecycle_task_tests.rs"]
mod batches_legacy_lifecycle_task_tests;
#[cfg(test)]
pub(super) use batches_legacy_lifecycle_task_tests::LEGACY_SKILL;
use serde_json::{json,Value};
use std::path::{Path,PathBuf};
use super::managed;
pub(super) const SKILL:&str=include_str!("assets/SKILL.md");
pub(super) const PLUGIN:&str="smart-explorer";
pub(super) const MARKETPLACE:&str="smart-explorer";
pub(super) const CLAUDE_SERVER:&str="se";
const MARKER:&str=".se-generated.json";
const DESCRIPTION:&str="Smart Explorer for AI agents: use its tools first, respect permissions, and follow the running server's instructions.";
const LEGACY_SKILL_SHA256:&str="6d23270eb3dff8c2a3845e06106c75db3808bee4bd72bdc43638eb9f3bf92193";

pub(super) fn executable()->Result<PathBuf,String> {
    match crate::mcp_launcher::Installation::current()? {
        Some(installation)=>installation.checked_launcher_path(),
        None=>crate::mcp_launcher::legacy_entry_path(),
    }
}
pub(super) fn claude_marketplace_dir()->PathBuf {crate::support_dirs::app_data_dir().join("ai-plugins/claude")}
pub(super) fn codex_skill_dir()->Option<PathBuf> {crate::support_dirs::home_dir().map(|home|home.join(".agents/skills").join(PLUGIN))}
pub(super) fn installation_for_registration(executable:&Path)->Result<Option<crate::mcp_launcher::Installation>,String> {
    let installation=crate::mcp_launcher::Installation::for_image(executable)?;
    if let Some(installation)=&installation {
        if !installation.registered_launcher_allowed(executable)? {return Err("integration_launcher_changed: retry with the current installation".into());}
    } else if executable!=crate::mcp_launcher::legacy_entry_path()? {return Err("integration_legacy_entry_mismatch".into());}
    Ok(installation)
}
fn marker(executable:&Path)->Result<Value,String> {
    if let Some(installation)=installation_for_registration(executable)? {
        Ok(json!({"schema":1,"installation_id":installation.manifest.installation_id,"launcher_path":executable,"version":installation.manifest.product_version,"executable":executable}))
    } else {
        Ok(json!({"schema":1,"mode":"legacy_cli","launcher_path":executable,"version":env!("CARGO_PKG_VERSION"),"executable":executable}))
    }
}
pub(super) fn write_claude(executable:&Path)->Result<PathBuf,String> {
    let root=claude_marketplace_dir();
    let plugin=PLUGIN;
    let mut mcp=json!({"mcpServers":{CLAUDE_SERVER:{"command":executable,"args":["mcp"]}}});
    let mcp_name=format!("{plugin}/.mcp.json");
    if let Some(bytes)=managed::read(&root.join(&mcp_name))? {
        let mut existing:Value=serde_json::from_slice(&bytes).map_err(|e|format!("integration_conflict: {}: {e}",root.join(&mcp_name).display()))?;
        if let Some(old)=read_marker(&root) {
            if !own_fields_match(&mcp_name,&bytes,&old) {return Err(format!("integration_conflict: {} command/args",root.join(&mcp_name).display()));}
            existing["mcpServers"][CLAUDE_SERVER]["command"]=json!(executable);
            existing["mcpServers"][CLAUDE_SERVER]["args"]=json!(["mcp"]);
            mcp=existing;
        }
    }
    let json_bytes=|v:Value|serde_json::to_vec_pretty(&v).map_err(|e|e.to_string());
    let files=vec![
        (".claude-plugin/marketplace.json".into(),json_bytes(json!({"name":MARKETPLACE,"owner":{"name":"Smart Explorer"},"plugins":[{"name":PLUGIN,"source":format!("./{PLUGIN}"),"description":DESCRIPTION}]}))?),
        (format!("{plugin}/.claude-plugin/plugin.json"),json_bytes(json!({"name":PLUGIN,"displayName":"Smart Explorer","version":env!("CARGO_PKG_VERSION"),"description":DESCRIPTION,"author":{"name":"Smart Explorer"},"homepage":"https://github.com/b1ue-man/smart-explorer","license":"MIT"}))?),
        (mcp_name,json_bytes(mcp)?),
        (format!("{plugin}/skills/{PLUGIN}/SKILL.md"),SKILL.as_bytes().to_vec()),
    ];
    managed::write_set(&root,&files,marker(executable)?)?;Ok(root)
}
pub(super) fn write_codex_skill(executable:&Path)->Result<PathBuf,String> {
    let directory=codex_skill_dir().ok_or("no home directory for Codex skill")?;
    managed::write_set(&directory,&[("SKILL.md".into(),SKILL.as_bytes().to_vec())],marker(executable)?)?;
    Ok(directory)
}
pub(super) fn read_marker(directory:&Path)->Option<Value> {
    serde_json::from_slice(&std::fs::read(directory.join(MARKER)).ok()?).ok()
}
/// An earlier selective removal may leave empty directories or user notes.
/// Their existence alone is neither ownership nor a conflict with retry.
pub(super) fn generated_files_absent(root:&Path)->Result<bool,String> {
    for name in [MARKER,"SKILL.md",".claude-plugin/marketplace.json","smart-explorer/.claude-plugin/plugin.json","smart-explorer/.mcp.json","smart-explorer/skills/smart-explorer/SKILL.md"] {
        match std::fs::symlink_metadata(root.join(name)) {
            Ok(_)=>return Ok(false),Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},Err(error)=>return Err(error.to_string()),
        }
    }
    Ok(true)
}
pub(super) fn verify_owned(directory:&Path)->Result<(),String> {
    let marker=read_marker(directory).ok_or_else(||format!("integration_conflict: {} ownership marker missing",directory.display()))?;
    verify_marker_owned(directory,&marker)
}
fn verify_marker_owned(directory:&Path,marker:&Value)->Result<(),String> {
    if marker.get("schema").is_none() {return legacy_owned_files(directory,&marker).map(|_|());}
    if !has_fingerprints(&marker) {return Err("integration_conflict: ownership fingerprints missing".into());}
    if let Some(files)=marker["fingerprints"].as_object() {
        for (name,expected) in files {verify_owned_file(directory,name,expected,false)?;}
    }
    if let Some(fields)=marker["field_fingerprints"].as_object() {
        for (name,expected) in fields {verify_owned_file(directory,name,expected,true)?;}
    }
    Ok(())
}
fn verify_owned_file(root:&Path,name:&str,expected:&Value,fields:bool)->Result<(),String> {
    if !managed_relative(name) {return Err("integration_conflict: unknown managed file path".into());}
    let path=root.join(name);
    let bytes=managed::read(&path)?.ok_or_else(||format!("integration_conflict: {} missing",path.display()))?;
    let actual=if fields {owned_fields_fingerprint(name,&bytes)}else{Some(managed::hash(&bytes))};
    if actual.as_deref()!=expected.as_str() {return Err(format!("integration_conflict: {} owned content changed",path.display()));}
    // Even an older marker may have accidentally fingerprinted a complete
    // shared MCP document. Never turn that fingerprint into deletion rights.
    if name.ends_with("/.mcp.json") && !mcp_wholly_owned(&bytes) {
        return Err(format!("integration_conflict: {} contains user fields or additional MCP servers; uninstall preserves this file and requires explicit cleanup",path.display()));
    }
    Ok(())
}
pub(super) fn remove_generated(directory:&Path)->Result<bool,String> {
    if generated_files_absent(directory)? {return Ok(false);}
    let _lock=managed::lock()?;
    remove_owned_files(directory)
}
fn remove_owned_files(directory:&Path)->Result<bool,String> {
    let marker=read_marker(directory).ok_or("integration_conflict: ownership marker missing")?;
    verify_marker_owned(directory,&marker)?;
    let mut names=std::collections::BTreeSet::new();
    if marker.get("schema").is_none() {names.extend(legacy_owned_files(directory,&marker)?);} else {
        for group in ["fingerprints","field_fingerprints"] {
            if let Some(files)=marker[group].as_object() {for name in files.keys() {names.insert(name.clone());}}
        }
    }
    if names.iter().any(|name|!managed_relative(name)) {return Err("integration_conflict: unknown managed file path".into());}
    for name in names {std::fs::remove_file(directory.join(name)).map_err(|e|e.to_string())?;}
    std::fs::remove_file(directory.join(MARKER)).map_err(|e|e.to_string())?;Ok(true)
}
pub(super) fn generated_state(directory:Option<PathBuf>,executable:&Path)->Value {
    let Some(directory)=directory else {return json!({"generated":false});};
    let Some(marker)=read_marker(&directory) else {return json!({"generated":false,"path":directory});};
    let mut conflicts=Vec::new();
    for group in ["fingerprints","field_fingerprints"] {
        if let Some(files)=marker[group].as_object() {for (name,expected) in files {
            let path=directory.join(name);
            if !managed_relative(name) {conflicts.push(path);continue;}
            let bytes=managed::read(&path).ok().flatten();
            let actual=bytes.as_deref().and_then(|bytes|if group=="field_fingerprints" {owned_fields_fingerprint(name,bytes)}else{Some(managed::hash(bytes))});
            if actual.as_deref()!=expected.as_str() {conflicts.push(path);}
        }}
    }
    json!({"generated":true,"path":directory,"version":marker["version"],"current":is_current(&marker,executable)&&conflicts.is_empty(),"integration_conflict":conflicts,"legacy_migration_required":marker.get("schema").is_none()})
}
fn is_current(marker:&Value,executable:&Path)->bool {
    let Ok(installation)=crate::mcp_launcher::Installation::for_image(executable) else {return false;};
    let Some(installation)=installation else {
        return marker["schema"]==1 && marker["mode"]=="legacy_cli" && has_fingerprints(marker)
            && marker["version"]==env!("CARGO_PKG_VERSION") && marker["executable"]==json!(executable);
    };
    marker["schema"]==1 && has_fingerprints(marker) && marker["installation_id"].as_str()==Some(installation.manifest.installation_id.as_str())
        && marker["version"].as_str()==Some(installation.manifest.product_version.as_str())
        && marker["launcher_path"].as_str().is_some_and(|path|installation.registered_launcher_allowed(Path::new(path)).unwrap_or(false))
}
fn refresh_executable(root:&Path,current:&Path)->Result<PathBuf,String> {
    let Some(marker)=read_marker(root) else {return Ok(current.into());};
    let installation=crate::mcp_launcher::Installation::for_image(current)?;
    if marker["schema"]==1 {
        if !has_fingerprints(&marker) {return Err("integration_conflict: ownership fingerprints missing".into());}
        if marker["mode"]!="legacy_cli" && !installation.as_ref().is_some_and(|installation|marker["installation_id"].as_str()==Some(installation.manifest.installation_id.as_str())) {return Err("integration_conflict: profile belongs to a different installation".into());}
        // A marker alone is insufficient: verify every recorded own fingerprint
        // without treating permitted env/server additions as full-file ownership.
        for group in ["fingerprints","field_fingerprints"] {
            if let Some(files)=marker[group].as_object() {for (name,expected) in files {
                if !managed_relative(name) {return Err("integration_conflict: unknown managed path".into());}
                let bytes=managed::read(&root.join(name))?.ok_or("integration_conflict: managed file removed")?;
                let actual=if group=="field_fingerprints" {owned_fields_fingerprint(name,&bytes)}else{Some(managed::hash(&bytes))};
                if actual.as_deref()!=expected.as_str() && !own_fields_match(name,&bytes,&marker) {return Err(format!("integration_conflict: {} changed",root.join(name).display()));}
            }}
        }
        if let (Some(installation),Some(path))=(installation.as_ref(),marker["launcher_path"].as_str()) {
            let path=Path::new(path);
            if installation.registered_launcher_allowed(path)? {return Ok(path.into());}
        }
    }
    Ok(current.into())
}
pub(super) fn refresh()->Result<(),String> {
    let _generator=crate::mcp_launcher::generator_operation()?;
    let _legacy_update=crate::mcp_launcher::legacy_profile_guard()?;
    let executable=executable()?;
    let claude=claude_marketplace_dir();
    if read_marker(&claude).is_some() {write_claude(&refresh_executable(&claude,&executable)?)?;}
    if let Some(skill)=codex_skill_dir().filter(|path|read_marker(path).is_some()) {
        super::codex::refresh(&refresh_executable(&skill,&executable)?)?;
    }
    Ok(())
}
pub(super) fn own_fields_match(name:&str,bytes:&[u8],marker:&Value)->bool {
    if !name.ends_with("/.mcp.json") {return false;}
    let Ok(value)=serde_json::from_slice::<Value>(bytes) else {return false;};
    let expected=marker["launcher_path"].as_str().or_else(||marker["executable"].as_str());
    value["mcpServers"][CLAUDE_SERVER]["command"].as_str()==expected
        && value["mcpServers"][CLAUDE_SERVER]["args"]==json!(["mcp"])
}
pub(super) fn legacy_file_matches(name:&str,bytes:&[u8],marker:&Value)->bool {
    if name.ends_with("SKILL.md") {return managed::hash(bytes)==LEGACY_SKILL_SHA256;}
    if name.ends_with("/.mcp.json") {return own_fields_match(name,bytes,marker);}
    let Ok(value)=serde_json::from_slice::<Value>(bytes) else {return false;};
    let description="Smart Explorer for AI agents, used before the built-in file tools: files on local disks, saved remotes (SFTP, FTP/FTPS, WebDAV, SMB), Share peers and Google Drive, and remote commands over SSH or Smart Explorer Share.";
    if name==".claude-plugin/marketplace.json" {
        value==json!({"name":MARKETPLACE,"owner":{"name":"Smart Explorer"},"plugins":[{"name":PLUGIN,"source":format!("./{PLUGIN}"),"description":description}]})
    } else if name.ends_with("/.claude-plugin/plugin.json") {
        value==json!({"name":PLUGIN,"displayName":"Smart Explorer","version":marker["version"],"description":description,"author":{"name":"Smart Explorer"},"homepage":"https://github.com/b1ue-man/smart-explorer","license":"MIT"})
    } else {false}
}

fn managed_relative(name:&str)->bool {matches!(name,"SKILL.md"|".claude-plugin/marketplace.json"|"smart-explorer/.claude-plugin/plugin.json"|"smart-explorer/.mcp.json"|"smart-explorer/skills/smart-explorer/SKILL.md")}

/// Only command/args belong to Smart Explorer in this shared document.
pub(super) fn owned_fields_fingerprint(name:&str,bytes:&[u8])->Option<String> {
    if !name.ends_with("/.mcp.json") {return None;}
    let value:Value=serde_json::from_slice(bytes).ok()?;
    let server=&value["mcpServers"][CLAUDE_SERVER];
    let owned=json!({"command":server.get("command")?,"args":server.get("args")?});
    Some(managed::hash(&serde_json::to_vec(&owned).ok()?))
}
fn mcp_wholly_owned(bytes:&[u8])->bool {
    let Ok(value)=serde_json::from_slice::<Value>(bytes) else {return false;};
    let server=&value["mcpServers"][CLAUDE_SERVER];
    server["command"].is_string() && server["args"]==json!(["mcp"])
        && value==json!({"mcpServers":{CLAUDE_SERVER:{"command":server["command"],"args":["mcp"]}}})
}

fn has_fingerprints(marker:&Value)->bool {
    ["fingerprints","field_fingerprints"].iter().any(|group|marker[*group].as_object().is_some_and(|files|!files.is_empty()))
}

/// The released marker had no fingerprints. Accept only the exact published
/// file set and content, never use that marker as recursive deletion rights.
fn legacy_owned_files(root:&Path,marker:&Value)->Result<Vec<String>,String> {
    if marker.as_object().is_none_or(|marker|marker.len()!=2) || marker.get("schema").is_some() || marker["version"].as_str().is_none_or(str::is_empty)
        || !marker["executable"].as_str().is_some_and(|value|Path::new(value).is_absolute()) {
        return Err("integration_conflict: legacy ownership marker invalid".into());
    }
    let names:&[&str]=if root.join(".claude-plugin/marketplace.json").exists() {
        &[".claude-plugin/marketplace.json","smart-explorer/.claude-plugin/plugin.json",
          "smart-explorer/.mcp.json","smart-explorer/skills/smart-explorer/SKILL.md"]
    } else {&["SKILL.md"]};
    for name in names {
        let bytes=managed::read(&root.join(name))?.ok_or_else(||format!("integration_conflict: {} missing",root.join(name).display()))?;
        if !legacy_file_matches(name,&bytes,marker)
            || (name.ends_with("/.mcp.json") && !mcp_wholly_owned(&bytes)) {
            return Err(format!("integration_conflict: {} legacy owned content changed or contains user fields",root.join(name).display()));
        }
    }
    Ok(names.iter().map(|name|(*name).into()).collect())
}
