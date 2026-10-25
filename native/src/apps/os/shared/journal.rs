//! Durable request claims survive lost replies and host restarts. No eviction:
//! forgetting an accepted ID would permit a duplicate OS start.
use std::{fs, io::{self, Read, Write}, path::{Path, PathBuf}};
use sha2::{Digest, Sha256};
use crate::apps::core::model::{LaunchReceipt,LaunchRequest,RequestId};

const MAX_RECORD_BYTES: u64 = 32 * 1024;

pub(crate) struct Journal { root: PathBuf }
impl Journal {
    /// The caller supplies the host's private persistent app-data directory.
    /// An unavailable ledger is an error, never an in-memory fallback.
    pub(crate) fn new(root: &Path) -> io::Result<Self> {
        super::super::platform::prepare_journal(root)?;
        Ok(Self { root: root.to_owned() })
    }
    fn path(&self, id: &RequestId) -> PathBuf {
        self.root.join(format!("{:x}.json", Sha256::digest(id.as_str().as_bytes())))
    }
    /// Bind even a refused or temporarily unresolvable request to its exact ID.
    /// This file is separate from the admitted-operation receipt.
    pub(crate) fn bound_request(&self,id:&RequestId)->io::Result<Option<LaunchRequest>> {
        let path=self.root.join(format!("{:x}.identity",Sha256::digest(id.as_str().as_bytes())));
        let file=match fs::File::open(&path) {Ok(file)=>file,Err(e) if e.kind()==io::ErrorKind::NotFound=>return Ok(None),Err(e)=>return Err(e)};
        if fs::symlink_metadata(&path)?.file_type().is_symlink()||!file.metadata()?.is_file() {return Err(io::Error::other("launch identity is not a regular private file"));}
        let mut bytes=Vec::new();file.take(MAX_RECORD_BYTES+1).read_to_end(&mut bytes)?;
        if bytes.len() as u64>MAX_RECORD_BYTES {return Err(io::Error::other("launch identity exceeds byte budget"));}
        let request:LaunchRequest=serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if request.request_id!=*id {return Err(io::Error::other("launch identity ID mismatch"));}Ok(Some(request))
    }
    pub(crate) fn bind(&self,request:&LaunchRequest)->io::Result<bool> {
        let path=self.root.join(format!("{:x}.identity",Sha256::digest(request.request_id.as_str().as_bytes())));
        let check=||->io::Result<Option<bool>> {
            let file=match fs::File::open(&path) {Ok(f)=>f,Err(e) if e.kind()==io::ErrorKind::NotFound=>return Ok(None),Err(e)=>return Err(e)};
            if fs::symlink_metadata(&path)?.file_type().is_symlink() || !file.metadata()?.is_file() {return Err(io::Error::other("launch identity is not a regular private file"));}
            let mut bytes=Vec::new();file.take(MAX_RECORD_BYTES+1).read_to_end(&mut bytes)?;
            if bytes.len() as u64>MAX_RECORD_BYTES {return Err(io::Error::other("launch identity exceeds byte budget"));}
            let original:LaunchRequest=serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            Ok(Some(original==*request))
        };
        if let Some(same)=check()? {return Ok(same);}
        let bytes=serde_json::to_vec(request).map_err(io::Error::other)?;
        if bytes.len() as u64>MAX_RECORD_BYTES {return Err(io::Error::other("launch request exceeds byte budget"));}
        let mut file=tempfile::NamedTempFile::new_in(&self.root)?;file.write_all(&bytes)?;file.as_file().sync_all()?;
        if super::super::platform::publish_record(file,&path,false)? {super::super::platform::sync_journal(&self.root)?;return Ok(true);}
        check()?.ok_or_else(||io::Error::other("bound launch identity disappeared"))
    }
    pub(crate) fn read(&self, id: &RequestId) -> io::Result<Option<LaunchReceipt>> {
        let path = self.path(id);
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        if !file.metadata()?.is_file() || fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(io::Error::other("launch ledger entry is not a regular private file"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_RECORD_BYTES { return Err(io::Error::other("launch receipt exceeds byte budget")); }
        let receipt: LaunchReceipt = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if &receipt.request.request_id != id { return Err(io::Error::other("launch receipt identity mismatch")); }
        Ok(Some(receipt))
    }
    fn staged(&self, receipt: &LaunchReceipt) -> io::Result<tempfile::NamedTempFile> {
        let bytes = serde_json::to_vec(receipt).map_err(io::Error::other)?;
        if bytes.len() as u64 > MAX_RECORD_BYTES { return Err(io::Error::other("launch receipt exceeds byte budget")); }
        let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        Ok(file)
    }
    /// Atomic no-replace publication: precisely one process owns dispatch.
    pub(crate) fn claim(&self, receipt: &LaunchReceipt) -> io::Result<bool> {
        let claimed=super::super::platform::publish_record(self.staged(receipt)?,&self.path(&receipt.request.request_id),false)?;
        if claimed {super::super::platform::sync_journal(&self.root)?;}
        Ok(claimed)
    }
    pub(crate) fn update(&self, receipt: &LaunchReceipt) -> io::Result<()> {
        super::super::platform::publish_record(self.staged(receipt)?,&self.path(&receipt.request.request_id),true)?;
        super::super::platform::sync_journal(&self.root)
    }
    pub(crate) fn claim_continuation(&self, id: &RequestId) -> io::Result<bool> {
        let path=self.root.join(format!("{:x}.consent",Sha256::digest(id.as_str().as_bytes())));
        let mut file=tempfile::NamedTempFile::new_in(&self.root)?;
        file.write_all(b"local consent continuation claimed\n")?;
        file.as_file().sync_all()?;
        if !super::super::platform::publish_record(file,&path,false)? {return Ok(false);}
        super::super::platform::sync_journal(&self.root)?;
        Ok(true)
    }
    /// Only after a trusted OS adapter returned ConsentRequired without calling
    /// its OS start API. Unknown/started phases must retain their claim forever.
    pub(crate) fn release_unstarted_continuation(&self,id:&RequestId)->io::Result<()> {
        let path=self.root.join(format!("{:x}.consent",Sha256::digest(id.as_str().as_bytes())));
        let metadata=match fs::symlink_metadata(&path) {
            Ok(metadata)=>metadata,
            Err(e) if e.kind()==io::ErrorKind::NotFound=>return Ok(()),
            Err(e)=>return Err(e),
        };
        if !metadata.is_file()||metadata.file_type().is_symlink() {return Err(io::Error::other("invalid local consent claim"));}
        fs::remove_file(path)?;
        super::super::platform::sync_journal(&self.root)
    }
}
