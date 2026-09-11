//! Disk-backed private copy stages own a Drive ID, never an existing path ID.
//! This is not atomic name reservation: Drive permits duplicate sibling names.

use super::api::parse_generated_id;
use super::core::{cloud_urlenc, norm, split_parent};
use super::resumable;
use super::transfer::initiate;
use super::GDriveBackend;
use crate::vfs::VfsResult;
use std::fs::File;
use std::io::{self, Write};

#[cfg(test)]
#[path = "copy_writer_task_tests.rs"]
mod copy_writer_task_tests;

const MEDIA_TYPE: &str = "application/octet-stream";

pub(super) fn open_writer(backend: &GDriveBackend, path: &str) -> VfsResult<Box<dyn Write + Send>> {
    let path = norm(path);
    if path.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Drive copy stage must name a non-root file",
        ));
    }
    Ok(Box::new(CopyWriter {
        backend: backend.clone(),
        path,
        spool: Some(tempfile::tempfile()?),
        size: 0,
        md5: md5::Context::new(),
        state: CopyState::Open,
    }))
}

#[derive(Clone)]
struct OwnedStage {
    id: String,
    parent_id: String,
    name: String,
    size: u64,
    md5: String,
}

enum CopyState {
    Open,
    // Once a mutation could have started, this exact ID and content are frozen.
    // A later flush only reconciles; it never sends PATCH or creates another ID.
    Pending(OwnedStage),
    Committed,
}

struct CopyWriter {
    backend: GDriveBackend,
    path: String,
    spool: Option<File>,
    size: u64,
    md5: md5::Context,
    state: CopyState,
}

impl CopyWriter {
    fn commit(&mut self) -> io::Result<()> {
        if matches!(&self.state, CopyState::Committed) {
            return Ok(());
        }
        let _path_guard = self.backend.upload_path_guard(&self.path)?;
        if let CopyState::Pending(stage) = &self.state {
            let stage = stage.clone();
            return self.finish_verified(&stage);
        }

        let spool = self.spool.as_mut().ok_or_else(closed_spool)?;
        spool.flush()?;
        spool.sync_all()?;
        let (parent, name) = split_parent(&self.path);
        let parent_id = self.backend.ensure_dir(&parent)?;
        if !self.backend.named_objects(&parent_id, name)?.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Drive copy staging name is already occupied",
            ));
        }

        // Do not reuse pending_upload_ids or any cached/found path identity:
        // those may belong to another writer or an ordinary replacement upload.
        let generated = self.backend.get_json(
            &self.backend.api_url("files/generateIds?count=1&space=drive&type=files"),
        )?;
        let stage = OwnedStage {
            id: parse_generated_id(&generated)?,
            parent_id,
            name: name.to_string(),
            size: self.size,
            md5: format!("{:x}", self.md5.clone().compute()),
        };
        let bearer = format!("Bearer {}", self.backend.bearer()?);
        let metadata = serde_json::json!({
            "id": &stage.id,
            "name": &stage.name,
            "parents": [&stage.parent_id],
            "mimeType": MEDIA_TYPE,
        }).to_string();
        // Production api_base is Google's /drive/v3 endpoint. Deriving the
        // upload URI from that configured origin also keeps API fixtures local.
        let api_base = self.backend.api_base.trim_end_matches('/');
        let origin = api_base.strip_suffix("/drive/v3").unwrap_or(api_base);
        let upload_url = format!(
            "{origin}/upload/drive/v3/files?uploadType=resumable&fields=id"
        );
        self.state = CopyState::Pending(stage.clone());
        let uploaded = (|| {
            // Even a 409 or lost initiation response is reconciled below by
            // this generated ID. Never switch to PATCH when an ID is present.
            let session = initiate("POST", &upload_url, &bearer, stage.size, &metadata)?;
            resumable::upload(
                &session,
                self.spool.as_mut().ok_or_else(closed_spool)?,
                stage.size,
                &stage.id,
                || self.backend.bearer(),
                || self.backend.force_refresh_bearer(),
            )?;
            Ok::<(), io::Error>(())
        })();
        match (uploaded, self.finish_verified(&stage)) {
            (_, Ok(())) => Ok(()),
            (Ok(()), Err(verification)) => Err(verification),
            (Err(upload), Err(verification)) => Err(io::Error::new(
                verification.kind(),
                format!(
                    "Drive copy stage ID {} is unconfirmed; upload failed ({upload}); exact-ID verification failed ({verification}); no path cleanup was attempted",
                    stage.id
                ),
            )),
        }
    }

    fn finish_verified(&mut self, stage: &OwnedStage) -> io::Result<()> {
        let mime = verify_stage(&self.backend, stage)?;
        self.backend.remember_path(&self.path, &stage.id, Some(&mime))?;
        self.backend.persist_path_cache();
        self.state = CopyState::Committed;
        drop(self.spool.take());
        Ok(())
    }
}

impl Write for CopyWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if !matches!(&self.state, CopyState::Open) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Drive copy stage content is frozen after its first create attempt",
            ));
        }
        let requested = u64::try_from(data.len()).map_err(|_| size_overflow())?;
        self.size.checked_add(requested).ok_or_else(size_overflow)?;
        let written = self.spool.as_mut().ok_or_else(closed_spool)?.write(data)?;
        self.md5.consume(&data[..written]);
        self.size += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.commit()
    }
}

// File/Option drop only removes the anonymous local spool. It must not resolve
// or delete a remote staging path after collision or ambiguous completion.

fn verify_stage(backend: &GDriveBackend, stage: &OwnedStage) -> io::Result<String> {
    let url = backend.api_url(&format!(
        "files/{}?fields=id,name,parents,mimeType,size,md5Checksum,trashed",
        cloud_urlenc(&stage.id)
    ));
    let json = backend.get_json(&url)?;
    if !matches_stage(&json, stage) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Drive copy stage ID {} does not match its expected name, parent, size and checksum",
                stage.id
            ),
        ));
    }
    let objects = backend.named_objects(&stage.parent_id, &stage.name)?;
    if objects.len() != 1 || objects[0].id != stage.id {
        return Err(io::Error::new(
            if objects.is_empty() {
                io::ErrorKind::InvalidData
            } else {
                io::ErrorKind::AlreadyExists
            },
            format!("Drive copy staging name does not uniquely identify owned ID {}", stage.id),
        ));
    }
    let object = &objects[0];
    if object.size != Some(stage.size)
        || !object.md5.as_deref().is_some_and(|md5| md5.eq_ignore_ascii_case(&stage.md5))
        || !binary_mime(&object.mime_type)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Drive copy stage content changed before namespace verification",
        ));
    }
    Ok(object.mime_type.clone())
}

fn matches_stage(json: &serde_json::Value, stage: &OwnedStage) -> bool {
    json["id"].as_str() == Some(stage.id.as_str())
        && json["name"].as_str() == Some(stage.name.as_str())
        && json["trashed"].as_bool() == Some(false)
        && json["parents"].as_array().is_some_and(|parents| {
            parents.len() == 1 && parents[0].as_str() == Some(stage.parent_id.as_str())
        })
        && json["size"].as_str().and_then(|size| size.parse::<u64>().ok()) == Some(stage.size)
        && json["md5Checksum"].as_str().is_some_and(|md5| md5.eq_ignore_ascii_case(&stage.md5))
        && json["mimeType"].as_str().is_some_and(binary_mime)
}

fn binary_mime(mime: &str) -> bool {
    !mime.is_empty() && !mime.starts_with("application/vnd.google-apps.")
}

fn closed_spool() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "Drive copy spool is closed")
}

fn size_overflow() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "Drive copy stage exceeds supported size")
}
