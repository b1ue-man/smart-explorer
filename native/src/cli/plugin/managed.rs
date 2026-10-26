//! Compare-before-replace transactions for our own profile fields and files.
#[cfg(test)]
#[path = "batches_managed_task_tests.rs"]
mod batches_managed_task_tests;
use std::path::Path;
use std::io::Write;
use serde_json::Value;
use sha2::{Digest,Sha256};

pub(super) fn hash(bytes:&[u8])->String {format!("{:x}",Sha256::digest(bytes))}
pub(super) fn read(path:&Path)->Result<Option<Vec<u8>>,String> {
    match std::fs::read(path) {Ok(bytes)=>Ok(Some(bytes)),Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(None),Err(e)=>Err(format!("{}: {e}",path.display()))}
}
pub(super) fn replace(path:&Path,before:Option<&[u8]>,after:&[u8])->Result<(),String> {
    if read(path)?.as_deref()!=before {return Err(format!("integration_conflict: {} changed before replacement",path.display()));}
    let parent=path.parent().ok_or("integration_parent_missing")?;
    std::fs::create_dir_all(parent).map_err(|e|e.to_string())?;
    let mut staged=tempfile::NamedTempFile::new_in(parent).map_err(|e|e.to_string())?;
    staged.write_all(after).and_then(|()|staged.as_file().sync_all()).map_err(|e|e.to_string())?;
    if read(path)?.as_deref()!=before {return Err(format!("integration_conflict: {} changed during staging",path.display()));}
    staged.persist(path).map_err(|e|e.to_string())?;
    if read(path)?.as_deref()!=Some(after) {return Err(format!("integration_conflict: {} changed after replacement",path.display()));}
    Ok(())
}
pub(super) fn lock()->Result<std::fs::File,String> {
    let root=crate::support_dirs::app_data_dir().join("ai-plugins");
    crate::support_dirs::ensure_private_dir(&root).map_err(|e|e.to_string())?;
    let file=crate::support_dirs::open_private_lock(&root.join("integration.lock")).map_err(|e|e.to_string())?;
    file.try_lock().map_err(|e|format!("integration_busy: {e}"))?;Ok(file)
}
pub(super) fn write_set(root:&Path,files:&[(String,Vec<u8>)],mut marker:Value)->Result<(),String> {
    let _lock=lock()?;
    let old_marker=super::files::read_marker(root);
    if old_marker.as_ref().and_then(|m| m["installation_id"].as_str()).is_some_and(|id| Some(id) != marker["installation_id"].as_str()) {
        return Err(format!("integration_conflict: {} belongs to a different installation",root.display()));
    }
    let mut changes=Vec::new();
    let mut fingerprints=serde_json::Map::new();
    let mut field_fingerprints=serde_json::Map::new();
    for (name,after) in files {
        let path=root.join(name);let before=read(&path)?;
        if let Some(before)=&before {
            let expected=old_marker.as_ref().and_then(|m|m["fingerprints"][name].as_str());
            let legacy=old_marker.as_ref().is_some_and(|m|m.get("schema").is_none())
                && old_marker.as_ref().is_some_and(|m| super::files::legacy_file_matches(name,before,m));
            if expected!=Some(hash(before).as_str()) && !legacy && !old_marker.as_ref().is_some_and(|m| super::files::own_fields_match(name,before,m)) {return Err(format!("integration_conflict: {} fingerprint differs",path.display()));}
        } else if old_marker.as_ref().is_some_and(|m|m["fingerprints"].get(name).is_some() || m["field_fingerprints"].get(name).is_some()) {
            return Err(format!("integration_conflict: {} was removed",path.display()));
        }
        if let Some(fields)=super::files::owned_fields_fingerprint(name,after) {
            field_fingerprints.insert(name.clone(),Value::String(fields));
        }else {fingerprints.insert(name.clone(),Value::String(hash(after)));}
        changes.push((path,before,after.clone()));
    }
    marker["fingerprints"]=Value::Object(fingerprints);
    marker["field_fingerprints"]=Value::Object(field_fingerprints);
    let marker_path=root.join(".se-generated.json");
    changes.push((marker_path.clone(),read(&marker_path)?,serde_json::to_vec_pretty(&marker).map_err(|e|e.to_string())?));
    for (index,(path,before,after)) in changes.iter().enumerate() {
        if let Err(error)=replace(path,before.as_deref(),after) {
            // Undo only bytes still exactly ours, never an editor's newer file.
            for (path,old,new) in changes[..index].iter().rev() {
                if read(path).ok().flatten().as_deref()==Some(new.as_slice()) {
                    if let Some(old)=old {let _=replace(path,Some(new),old);} else {let _=std::fs::remove_file(path);}
                }
            }
            return Err(error);
        }
    }
    Ok(())
}
