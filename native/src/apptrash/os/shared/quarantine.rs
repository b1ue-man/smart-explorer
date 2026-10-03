//! Publish a captured regular file with the existing app-trash restore schema.
use std::{ffi::OsStr,io::{self,Write},path::Path};
use ring::rand::{SecureRandom,SystemRandom};
use crate::local_access::{DirectoryHandle,QuarantinedChild};
use super::{record::{Record,TrashEntry,entry_id,volume_for},store,TRASH_DIR_NAME};

pub(crate) fn publish_captured(captured:&mut QuarantinedChild,original:&Path)->io::Result<TrashEntry> {
    let volumes:Vec<_>=super::volumes().into_iter().filter_map(|p|std::fs::canonicalize(p).ok()).collect();
    let volume=volume_for(&volumes,original).ok_or_else(||io::Error::new(io::ErrorKind::Unsupported,"Dieser Ort hat keinen registrierten App-Papierkorb"))?;
    let root=volume.join(TRASH_DIR_NAME);
    if original.starts_with(&root) { return Err(io::Error::new(io::ErrorKind::PermissionDenied,"Papierkorbeintrag darf nicht erneut eingefangen werden")); }
    let name=original.file_name().and_then(|name|name.to_str()).ok_or_else(||io::Error::new(io::ErrorKind::InvalidInput,"Papierkorbname ist nicht darstellbar"))?;
    let volume_handle=DirectoryHandle::open_root(volume)?;
    let trash=directory(&volume_handle,OsStr::new(TRASH_DIR_NAME))?;
    let mut random=[0;8]; SystemRandom::new().fill(&mut random).map_err(|_|io::Error::other("Papierkorb-ID konnte nicht erzeugt werden"))?;
    let deleted_ms=store::now_ms(); let id=entry_id(deleted_ms,u64::from_le_bytes(random));
    let entry=TrashEntry { id:id.clone(),name:name.into(),original:original.into(),deleted_ms,
        size:captured.file().metadata()?.len(),is_dir:false };
    let record=Record::from_entry(&entry);
    if !record.is_trusted(&id,volume,&root) { return Err(io::Error::new(io::ErrorKind::InvalidInput,"Papierkorbrecord liegt außerhalb seines Volumes")); }
    let mut info=trash.create_file_new(OsStr::new(&format!("{id}.json")))?;
    let bytes=serde_json::to_vec(&record).map_err(io::Error::other)?;
    if bytes.len() as u64>super::record::MAX_RECORD_BYTES { return Err(io::Error::new(io::ErrorKind::InvalidInput,"Papierkorbrecord zu groß")); }
    info.write_all(&bytes)?; info.sync_all()?;
    let slot=trash.create_private_child(OsStr::new(&id))?;
    // No path-based rename or overwrite; the guard verifies the selected file.
    captured.move_to(&slot,OsStr::new(name))?;
    Ok(entry)
}
fn directory(parent:&DirectoryHandle,name:&OsStr)->io::Result<DirectoryHandle> {
    match parent.create_private_child(name) {
        Ok(directory)=>Ok(directory),Err(e) if e.kind()==io::ErrorKind::AlreadyExists=>parent.open_child(name),Err(e)=>Err(e),
    }
}
