use super::api::{drive_request, open_stream, API, UPLOAD};
use super::core::{cloud_urlenc, norm, split_parent};
use super::resumable::{self, Completion, REQUEST_TIMEOUT};
use super::GDriveBackend;
use crate::vfs::VfsResult;
use std::fs::File;
use std::io::{self, Write};

impl GDriveBackend {
    /// Upload a disk-backed spool through Drive's resumable protocol.
    fn upload_spooled(
        &self,
        path: &str,
        spool: &mut File,
        size: u64,
        expected_md5: &str,
    ) -> VfsResult<()> {
        let key = norm(path);
        let _upload_path_guard = self.upload_path_guard(&key)?;
        let (parent, name) = split_parent(&key);
        let parent_id = self.ensure_dir(&parent)?;
        let mut reserved = self.pending_upload_ids_guard()?.get(&key).cloned();
        // VFS staging names are an internal, hard-to-guess namespace and have
        // already been probed absent. Reserve their ID before the second probe:
        // if another object appeared in the meantime, fail instead of updating
        // that unrelated object and later deleting it as staging cleanup.
        if reserved.is_none() && is_internal_staging_path(&key) {
            reserved = Some(self.reserved_upload_id(&key)?);
        }
        let cached = if reserved.is_some() {
            None
        } else {
            self.valid_cached_id(&key)?
        };
        // Drive allows duplicate sibling names. The keyed upload guard keeps
        // this decision and commit serial for the same path while unrelated
        // file uploads remain parallel.
        let existing = match reserved.as_ref() {
            Some(reserved_id) => match self.find_child(&parent_id, name)? {
                Some(found_id) if &found_id != reserved_id => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "Drive upload retry found a different same-name file",
                    ));
                }
                Some(_) => Some(reserved_id.clone()),
                None => None,
            },
            None => match cached {
                Some(id) => Some(id),
                None => match self.valid_cached_id(&key)? {
                    Some(id) => Some(id),
                    None => self.find_child(&parent_id, name)?,
                },
            },
        };
        let creating = existing.is_none();
        let target_id = match existing.as_ref() {
            Some(id) => id.clone(),
            None => match reserved {
                Some(id) => id,
                None => self.reserved_upload_id(&key)?,
            },
        };
        let metadata = if creating {
            serde_json::json!({
                "id": target_id.clone(),
                "name": name,
                "parents": [parent_id],
            })
        } else {
            serde_json::json!({ "name": name })
        }
        .to_string();
        let auth = self.bearer()?;
        let bearer = format!("Bearer {auth}");
        let (method, init_url) = match existing.as_ref() {
            Some(id) => (
                "PATCH",
                format!(
                    "{UPLOAD}/{}?uploadType=resumable&fields=id",
                    cloud_urlenc(id)
                ),
            ),
            None => ("POST", format!("{UPLOAD}?uploadType=resumable&fields=id")),
        };
        let session = match initiate(method, &init_url, &bearer, size, &metadata) {
            Ok(location) => location,
            Err(error) => {
                // A retried create can receive 409 after the original request
                // actually succeeded. The pre-generated id makes verification
                // unambiguous and prevents a second same-name resource.
                if creating
                    && self
                        .uploaded_id_matches(&target_id, size, expected_md5)
                        .unwrap_or(false)
                {
                    self.remember_completed_upload(&key, &target_id)?;
                    return Ok(());
                }
                return Err(error);
            }
        };
        if resumable::upload(
            &session,
            spool,
            size,
            &target_id,
            || self.bearer(),
            || self.force_refresh_bearer(),
        )? == Completion::VerifyExpected
        {
            self.verify_uploaded_id(&target_id, size, expected_md5)?;
        }
        self.remember_completed_upload(&key, &target_id)
    }

    /// Replace the media of one known Drive object without taking a path lock.
    /// Callers must already serialize the destination path. Keeping this below
    /// `upload_spooled` avoids recursively acquiring the same keyed lock during
    /// staged promotion.
    pub(super) fn replace_spooled_id(
        &self,
        target_id: &str,
        spool: &mut File,
        size: u64,
        expected_md5: &str,
    ) -> VfsResult<()> {
        // A retry after the content commit but before staging cleanup can finish
        // without starting a second upload.
        if self.uploaded_id_matches(target_id, size, expected_md5)? {
            return Ok(());
        }

        let url = format!(
            "{UPLOAD}/{}?uploadType=resumable&fields=id",
            cloud_urlenc(target_id)
        );
        let bearer = format!("Bearer {}", self.bearer()?);
        let session = match initiate("PATCH", &url, &bearer, size, "{}") {
            Ok(location) => location,
            Err(error) => {
                if self
                    .uploaded_id_matches(target_id, size, expected_md5)
                    .unwrap_or(false)
                {
                    return Ok(());
                }
                return Err(error);
            }
        };
        resumable::upload(
            &session,
            spool,
            size,
            target_id,
            || self.bearer(),
            || self.force_refresh_bearer(),
        )?;
        // Promotion requires stronger confirmation than the ordinary writer:
        // the preserved destination ID, byte count, and checksum must all match
        // before the staging object can be removed.
        self.verify_uploaded_id(target_id, size, expected_md5)
    }

    fn generate_upload_id(&self) -> VfsResult<String> {
        let url = format!("{API}/files/generateIds?count=1&space=drive&type=files");
        parse_generated_id(&self.get_json(&url)?)
    }

    fn reserved_upload_id(&self, key: &str) -> VfsResult<String> {
        if let Some(id) = self.pending_upload_ids_guard()?.get(key).cloned() {
            return Ok(id);
        }
        let id = self.generate_upload_id()?;
        self.pending_upload_ids_guard()?
            .insert(key.to_string(), id.clone());
        Ok(id)
    }

    fn uploaded_id_matches(
        &self,
        id: &str,
        expected_size: u64,
        expected_md5: &str,
    ) -> VfsResult<bool> {
        let url = format!(
            "{API}/files/{}?fields=id,size,trashed,md5Checksum",
            cloud_urlenc(id)
        );
        Ok(uploaded_metadata_matches(
            &self.get_json(&url)?,
            id,
            expected_size,
            expected_md5,
        ))
    }

    fn verify_uploaded_id(
        &self,
        id: &str,
        expected_size: u64,
        expected_md5: &str,
    ) -> VfsResult<()> {
        if self.uploaded_id_matches(id, expected_size, expected_md5)? {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Drive could not verify the expected uploaded file id, size, and checksum",
            ))
        }
    }

    fn remember_completed_upload(&self, key: &str, id: &str) -> VfsResult<()> {
        self.remember_path(key, id, None)?;
        let mut pending = self.pending_upload_ids_guard()?;
        if pending.get(key).is_some_and(|pending_id| pending_id == id) {
            pending.remove(key);
        }
        drop(pending);
        self.persist_path_cache();
        Ok(())
    }

    pub(super) fn trash(&self, path: &str) -> VfsResult<()> {
        let key = norm(path);
        let _path_guard = self.upload_path_guard(&key)?;
        // An aborted create may not have entered the path cache. Prefer its
        // pre-generated ID so cleanup never resolves a racing same-name object.
        let pending_id = self.pending_upload_ids_guard()?.get(&key).cloned();
        let id = match pending_id.as_ref() {
            Some(id) => id.clone(),
            None => self.resolve(&key)?,
        };
        self.trash_id(&id)?;
        if pending_id.is_some() {
            self.pending_upload_ids_guard()?.remove(&key);
        }
        self.forget_path_prefix(&key);
        Ok(())
    }

    /// Trash one file by its exact id (targets a specific duplicate-named file).
    pub(super) fn trash_id(&self, id: &str) -> VfsResult<()> {
        let auth = self.bearer()?;
        let bearer = format!("Bearer {auth}");
        let url = format!("{API}/files/{}", cloud_urlenc(id));
        let payload = serde_json::json!({ "trashed": true }).to_string();
        open_stream(|| {
            drive_request(
                ureq::request("PATCH", &url)
                    .set("Authorization", &bearer)
                    .set("Content-Type", "application/json")
                    .send_string(&payload),
            )
        })?;
        Ok(())
    }
}

fn initiate(method: &str, url: &str, bearer: &str, size: u64, metadata: &str) -> VfsResult<String> {
    let agent = ureq::AgentBuilder::new().redirects(0).build();
    let size = size.to_string();
    let response = open_stream(|| {
        drive_request(
            agent
                .request(method, url)
                .timeout(REQUEST_TIMEOUT)
                .set("Authorization", bearer)
                .set("Content-Type", "application/json; charset=UTF-8")
                .set("X-Upload-Content-Type", "application/octet-stream")
                .set("X-Upload-Content-Length", &size)
                .send_string(metadata),
        )
    })?;
    initiation_location(response)
}

fn initiation_location(response: ureq::Response) -> VfsResult<String> {
    if response.status() != 200 {
        let status = response.status();
        let body = response.into_string().unwrap_or_default();
        return Err(io::Error::other(format!(
            "Drive resumable initiation returned HTTP {status}: {body}"
        )));
    }
    response
        .header("Location")
        .map(str::to_owned)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Drive resumable initiation has no Location header",
            )
        })
}

fn parse_generated_id(json: &serde_json::Value) -> VfsResult<String> {
    json["ids"]
        .as_array()
        .and_then(|ids| ids.first())
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Drive generateIds response has no usable id",
            )
        })
}

fn uploaded_metadata_matches(
    json: &serde_json::Value,
    id: &str,
    expected_size: u64,
    expected_md5: &str,
) -> bool {
    json["id"].as_str() == Some(id)
        && json["trashed"].as_bool() == Some(false)
        && json["size"]
            .as_str()
            .and_then(|size| size.parse::<u64>().ok())
            == Some(expected_size)
        && json["md5Checksum"].as_str() == Some(expected_md5)
}

fn is_internal_staging_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let Some((prefix, suffix)) = name.rsplit_once('-') else {
        return false;
    };
    let Some((_, purpose)) = prefix.rsplit_once(".se-") else {
        return false;
    };
    !purpose.is_empty() && suffix.len() == 16 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn open_writer(backend: &GDriveBackend, path: &str) -> VfsResult<Box<dyn Write + Send>> {
    Ok(Box::new(DriveWriter {
        backend: backend.clone(),
        path: norm(path),
        spool: Some(tempfile::tempfile()?),
        size: 0,
        md5: md5::Context::new(),
        committed: false,
    }))
}

struct DriveWriter {
    backend: GDriveBackend,
    path: String,
    spool: Option<File>,
    size: u64,
    md5: md5::Context,
    committed: bool,
}

impl DriveWriter {
    fn commit(&mut self) -> io::Result<()> {
        if self.committed {
            return Ok(());
        }
        let spool = self
            .spool
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "Drive spool is closed"))?;
        spool.flush()?;
        spool.sync_all()?;
        let expected_md5 = format!("{:x}", self.md5.clone().compute());
        self.backend
            .upload_spooled(&self.path, spool, self.size, &expected_md5)?;
        self.committed = true;
        drop(self.spool.take());
        Ok(())
    }
}

impl Write for DriveWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.committed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Drive writer is already committed",
            ));
        }
        let written = self
            .spool
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "Drive spool is closed"))?
            .write(data)?;
        self.md5.consume(&data[..written]);
        self.size = self.size.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.commit()
    }
}

impl Drop for DriveWriter {
    fn drop(&mut self) {
        // Drop is abort; only an explicit flush can publish the staged bytes.
        drop(self.spool.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_id_and_uploaded_metadata_are_strict() {
        let generated = serde_json::json!({"ids": ["known-id"]});
        assert_eq!(parse_generated_id(&generated).unwrap(), "known-id");
        assert!(parse_generated_id(&serde_json::json!({"ids": []})).is_err());

        let ok: ureq::Response =
            "HTTP/1.1 200 OK\r\nLocation: https://www.googleapis.com/upload/x\r\n\r\n"
                .parse()
                .unwrap();
        assert_eq!(
            initiation_location(ok).unwrap(),
            "https://www.googleapis.com/upload/x"
        );
        let wrong = ureq::Response::new(201, "Created", "").unwrap();
        assert!(initiation_location(wrong).is_err());

        let metadata = serde_json::json!({
            "id":"known-id", "size":"9", "trashed":false, "md5Checksum":"abc"
        });
        assert!(uploaded_metadata_matches(&metadata, "known-id", 9, "abc"));
        assert!(!uploaded_metadata_matches(&metadata, "other", 9, "abc"));
        assert!(!uploaded_metadata_matches(&metadata, "known-id", 8, "abc"));
        assert!(!uploaded_metadata_matches(&metadata, "known-id", 9, "def"));
    }

    #[test]
    fn internal_staging_names_are_narrowly_recognized() {
        assert!(is_internal_staging_path("file.se-bisync-0123456789abcdef"));
        assert!(!is_internal_staging_path("file.se-bisync-nothex"));
        assert!(!is_internal_staging_path("file-0123456789abcdef"));
    }
}
