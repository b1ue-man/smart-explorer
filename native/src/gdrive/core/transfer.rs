use super::api::{drive_request, parse_json, send_retry, API, UPLOAD};
use super::core::{norm, split_parent};
use super::GDriveBackend;
use crate::vfs::VfsResult;
use std::io::{self, Write};

impl GDriveBackend {
    /// Upload bytes to `path` (create or update). Used by `DriveWriter::flush`.
    pub(super) fn upload(&self, path: &str, data: &[u8]) -> VfsResult<()> {
        let key = norm(path);
        let (parent, name) = split_parent(&key);
        let parent_id = self.ensure_dir(&parent)?;
        // Existence: a cached id means update; otherwise, if the parent's
        // children are fully known (first sync into a fresh/empty folder), a
        // missing cache entry means it's a new file -> create without the extra
        // existence probe (one fewer round-trip per file across 27k files).
        let existing = match self.valid_cached_id(&key)? {
            Some(id) => Some(id),
            None => {
                if self.listed_guard()?.contains(&parent) {
                    None
                } else {
                    self.find_child(&parent_id, name)?
                }
            }
        };
        let boundary = "se_boundary_4f8a2c1d";
        let meta = if existing.is_some() {
            serde_json::json!({ "name": name })
        } else {
            serde_json::json!({ "name": name, "parents": [parent_id] })
        };
        let mut body: Vec<u8> = Vec::with_capacity(data.len() + 256);
        let head = format!(
            "--{b}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{m}\r\n--{b}\r\nContent-Type: application/octet-stream\r\n\r\n",
            b = boundary,
            m = meta
        );
        body.extend_from_slice(head.as_bytes());
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());

        let auth = self.bearer()?;
        let bearer = format!("Bearer {}", auth);
        let ct = format!("multipart/related; boundary={}", boundary);
        let v = match &existing {
            Some(id) => {
                let url = format!("{}/{}?uploadType=multipart&fields=id", UPLOAD, id);
                parse_json(send_retry(|| {
                    drive_request(
                        ureq::request("PATCH", &url)
                            .set("Authorization", &bearer)
                            .set("Content-Type", &ct)
                            .send_bytes(&body),
                    )
                })?)?
            }
            None => {
                let url = format!("{}?uploadType=multipart&fields=id", UPLOAD);
                parse_json(send_retry(|| {
                    drive_request(
                        ureq::post(&url)
                            .set("Authorization", &bearer)
                            .set("Content-Type", &ct)
                            .send_bytes(&body),
                    )
                })?)?
            }
        };
        let id = v["id"]
            .as_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Drive-Upload ohne id"))?;
        self.remember_path(&key, id, None)?;
        self.persist_path_cache();
        Ok(())
    }

    pub(super) fn trash(&self, path: &str) -> VfsResult<()> {
        let id = self.resolve(path)?;
        self.trash_id(&id)?;
        self.forget_path_prefix(&norm(path));
        Ok(())
    }

    /// Trash one file by its exact id (targets a specific duplicate-named file).
    pub(super) fn trash_id(&self, id: &str) -> VfsResult<()> {
        let auth = self.bearer()?;
        let bearer = format!("Bearer {}", auth);
        let url = format!("{}/files/{}", API, id);
        let payload = serde_json::json!({ "trashed": true }).to_string();
        send_retry(|| {
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

pub(super) fn open_writer(backend: &GDriveBackend, path: &str) -> Box<dyn Write + Send> {
    Box::new(DriveWriter {
        backend: backend.clone(),
        path: norm(path),
        buf: Vec::new(),
        done: false,
    })
}

/// Buffers written bytes and uploads to Drive on `flush` (so bisync's
/// `copy_between`, which flushes, surfaces upload errors) - and as a safety net
/// on drop if flush was never called.
struct DriveWriter {
    backend: GDriveBackend,
    path: String,
    buf: Vec<u8>,
    done: bool,
}

impl DriveWriter {
    fn flush_upload(&mut self) -> io::Result<()> {
        if self.done {
            return Ok(());
        }
        self.backend.upload(&self.path, &self.buf)?;
        self.done = true;
        Ok(())
    }
}

impl Write for DriveWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_upload()
    }
}

impl Drop for DriveWriter {
    fn drop(&mut self) {
        let _ = self.flush_upload();
    }
}
