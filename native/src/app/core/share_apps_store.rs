//! Client recovery records contain requests, never trusted host/window evidence.
use std::{io::{self,Write},path::PathBuf};
use serde::{Serialize,Deserialize};
use sha2::{Digest,Sha256};
use crate::{apps::LaunchRequest,share::PeerOpenTarget};

#[derive(Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Saved {
    pub target:PeerOpenTarget,
    pub request:LaunchRequest,
    pub name:String,
}
fn directory(target:&PeerOpenTarget)->PathBuf {
    crate::support_dirs::app_data_dir().join("app-start-ui")
        .join(format!("{:x}",Sha256::digest(target.endpoint_prefix().as_bytes())))
}
pub(super) fn read(target:&PeerOpenTarget)->Result<Option<Saved>,String> {
    let text=match crate::support_dirs::read_private_text(&directory(target).join("latest.json"),32*1024) {
        Ok(text)=>text,Err(error) if error.kind()==io::ErrorKind::NotFound=>return Ok(None),
        Err(error)=>return Err(format!("Gespeicherter Startauftrag nicht lesbar: {error}")),
    };
    let saved:Saved=serde_json::from_str(&text).map_err(|e|format!("Gespeicherter Startauftrag ist ungültig: {e}"))?;
    if saved.target!=*target {return Err("Gespeicherter Startauftrag gehört zu einer abweichenden Gerätebeziehung.".into());}
    Ok(Some(saved))
}
/// Store and sync the full immutable request before any launch IPC. The latest
/// pointer is convenience only; every explicitly created request remains stored.
pub(super) fn save(saved:&Saved)->Result<(),String> {
    let root=directory(&saved.target);
    let bytes=serde_json::to_vec(saved).map_err(|e|e.to_string())?;
    if bytes.len()>32*1024 {return Err("Startauftrag überschreitet das private Speicherlimit.".into());}
    let path=root.join(format!("{:x}.json",Sha256::digest(saved.request.request_id.as_str().as_bytes())));
    let write=(||->io::Result<()> {
        match crate::support_dirs::create_private_file(&path) {
            Ok(mut file)=>{file.write_all(&bytes)?;file.sync_all()?;},
            Err(e) if e.kind()==io::ErrorKind::AlreadyExists=>{
                let existing:Saved=serde_json::from_str(&crate::support_dirs::read_private_text(&path,32*1024)?).map_err(io::Error::other)?;
                if existing.request!=saved.request||existing.target!=saved.target {
                    return Err(io::Error::other("request ID is already bound to a different request"));
                }
            },
            Err(e)=>return Err(e),
        }
        crate::support_dirs::write_private_atomic(&root.join("latest.json"),&bytes)
    })();
    write.map_err(|e|format!("Startauftrag wurde nicht sicher gespeichert; kein Start versandt: {e}"))
}
