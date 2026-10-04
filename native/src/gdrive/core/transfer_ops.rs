//! Transfer-engine operations of the Drive backend (plan C4): exclusive folder
//! creation under a reserved ID, discarding an own copy stage by exact ID,
//! resuming a download by byte range and server-side copies into a stage.
use super::api::{export_format, FOLDER_MIME};
use super::core::{cloud_urlenc, norm, split_parent};
use super::new_object::NewObject;
use super::overload::http_status;
use super::GDriveBackend;
use crate::vfs::VfsResult;
use std::io::{self, Read};

impl GDriveBackend {
    /// Remember a stage ID this backend generated for `key`.
    pub(super) fn own_stage(&self, key: &str, id: &str) -> io::Result<()> {
        self.owned_stages_guard()?
            .insert(key.to_string(), id.to_string());
        Ok(())
    }

    /// The stage at `key` was published or discarded.
    pub(super) fn forget_owned_stage(&self, key: &str) {
        if let Ok(mut owned) = self.owned_stages_guard() {
            owned.remove(key);
        }
    }

    /// The MIME type of `id` at `path`: from the listing cache when the path
    /// still maps to that exact ID (no extra request per download), else
    /// asked by ID.
    pub(super) fn mime_for(&self, path: &str, id: &str) -> Option<String> {
        let key = norm(path);
        let same_id = self
            .cached_id(&key)
            .ok()
            .flatten()
            .is_some_and(|cached| cached == id);
        let cached = if same_id {
            self.mimes_guard()
                .ok()
                .and_then(|mimes| mimes.get(&key).cloned())
        } else {
            None
        };
        cached.or_else(|| self.mime_of_id(id))
    }

    /// `create_dir_new`: one new folder, `AlreadyExists` when the name is
    /// taken (by a folder or a file). Drive has no exclusive create, so the
    /// slot lock excludes this process, the reserved ID excludes duplicates of
    /// our own attempts, and a check after creating catches a concurrent
    /// client: then our (empty) folder goes to the trash again.
    pub(super) fn create_dir_exclusive(&self, path: &str) -> VfsResult<()> {
        let key = norm(path);
        if key.is_empty() {
            return Err(taken(&key));
        }
        let (parent, name) = split_parent(&key);
        let title = super::names::decode(name)?;
        let parent_id = self.resolve(&parent)?;
        let _slot = self.create_slot_guard(&parent_id, name)?;
        // An earlier attempt left a reservation for this path: settle it. The
        // folder it made (if any) is not new to this call, so never adopt it.
        if let Some(pending) = self.pending_folder_create(&key)? {
            self.resume_pending_folder_create(&parent, &key, name, &parent_id, &pending)?;
            return Err(taken(&key));
        }
        // A tombstone still reserves the locator; it cannot be reused by an
        // unrelated folder after cache clearing or a remote deletion.
        if self.bound_folder(&parent_id, name)?.is_some() {
            return Err(taken(&key));
        }
        if !self.named_objects(&parent_id, &title)?.is_empty() {
            return Err(taken(&key));
        }
        let id = self.take_generated_id()?;
        let (pending, newly_claimed) =
            self.reserve_pending_folder_create(&key, &id, &title, &parent_id)?;
        if !newly_claimed {
            self.resume_pending_folder_create(&parent, &key, name, &parent_id, &pending)?;
            return Err(taken(&key));
        }
        let created = self.submit_reserved_folder(&parent, &key, &pending, None)?;
        let objects = self.named_objects(&parent_id, &title)?;
        if objects.len() == 1 && objects[0].id == created {
            return Ok(());
        }
        // Someone else made the same name meanwhile: keep the namespace
        // unambiguous and report the name as taken.
        let cleanup = self.trash_id(&created);
        self.forget_path_prefix(&key);
        self.listed_guard()?.remove(&key);
        cleanup?;
        Err(taken(&key))
    }

    /// `discard_copy_stage`: trash a stage this backend created, by its exact
    /// ID, and only while it still lies under its stage name (a stage that a
    /// promotion may have published is never touched).
    pub(super) fn discard_stage(&self, stage: &str) -> VfsResult<()> {
        let key = norm(stage);
        let _path = self.upload_path_guard(&key)?;
        let Some(id) = self.owned_stages_guard()?.get(&key).cloned() else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Diese Drive-Stufe stammt nicht von dieser Verbindung und wird nicht entfernt",
            ));
        };
        let url = self.api_url(&format!(
            "files/{}?fields=id,name,parents,trashed",
            cloud_urlenc(&id)
        ));
        match self.get_json(&url) {
            // The create never happened: nothing to remove.
            Err(error) if http_status(&error) == Some(404) => {}
            Err(error) => return Err(error),
            Ok(json) if json["trashed"].as_bool() == Some(true) => {}
            Ok(json) => {
                let (parent, name) = split_parent(&key);
                let title = super::names::decode(name)?;
                let parent_id = self.resolve(&parent)?;
                let at_stage = json["name"].as_str() == Some(title.as_str())
                    && json["parents"].as_array().is_some_and(|parents| {
                        parents.len() == 1 && parents[0].as_str() == Some(parent_id.as_str())
                    });
                if !at_stage {
                    self.forget_owned_stage(&key);
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "Die Drive-Stufe liegt nicht mehr unter ihrem Stufennamen und wird nicht entfernt",
                    ));
                }
                self.trash_id(&id)?;
            }
        }
        self.forget_owned_stage(&key);
        if self.cached_id(&key)?.as_deref() == Some(id.as_str()) {
            self.forget_path_prefix(&key);
        }
        Ok(())
    }

    /// `open_read_at`: continue a download at `offset` with an HTTP range.
    /// Exported Google documents have no stable bytes to resume (`None`).
    pub(super) fn read_from(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        let id = match id.filter(|id| !id.is_empty()) {
            Some(id) => id.to_string(),
            None => self.resolve(path)?,
        };
        let mime = self.mime_for(path, &id).unwrap_or_default();
        if export_format(&mime).is_some() || mime == FOLDER_MIME {
            return Ok(None);
        }
        let url = self.api_url(&format!("files/{}?alt=media", cloud_urlenc(&id)));
        let range = format!("bytes={offset}-");
        let response = self.authenticated_stream(&url, Some(&range))?;
        match response.status() {
            206 => {
                let starts_at_offset = response
                    .header("Content-Range")
                    .and_then(|value| value.strip_prefix("bytes "))
                    .and_then(|value| value.split('-').next())
                    .and_then(|start| start.trim().parse::<u64>().ok())
                    == Some(offset);
                if !starts_at_offset {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Drive lieferte einen anderen Dateiabschnitt als angefragt",
                    ));
                }
                Ok(Some(Box::new(response.into_reader())))
            }
            // The server ignored the range: skip to the offset in the stream.
            200 => {
                let mut reader = response.into_reader();
                let skipped = io::copy(&mut reader.by_ref().take(offset), &mut io::sink())?;
                if skipped != offset {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Die Drive-Datei ist kürzer als der Fortsetzungspunkt",
                    ));
                }
                Ok(Some(Box::new(reader)))
            }
            status => Err(io::Error::other(format!(
                "Drive-Download ab Byte {offset}: unerwartete Antwort HTTP {status}"
            ))),
        }
    }

    /// `server_copy_to_stage`: `files.copy` of a binary file into a new stage
    /// under a reserved ID, so no byte leaves Google. Google documents keep
    /// streaming as an export (no fixed bytes, no reserved IDs for them).
    pub(super) fn copy_to_stage(
        &self,
        src: &str,
        stage: &str,
        size: u64,
    ) -> VfsResult<Option<u64>> {
        let source = norm(src);
        let key = norm(stage);
        if source.is_empty() || key.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Drive-Kopie braucht Quelle und Ziel unterhalb der Wurzel",
            ));
        }
        let source_id = self.resolve(&source)?;
        let mime = self.mime_for(&source, &source_id).unwrap_or_default();
        if mime.is_empty()
            || mime == FOLDER_MIME
            || mime.starts_with("application/vnd.google-apps.")
        {
            return Ok(None);
        }
        let (parent, name) = split_parent(&key);
        let title = super::names::decode(name)?;
        let parent_id = self.ensure_dir(&parent)?;
        let object = NewObject {
            id: self.take_generated_id()?,
            parent_id,
            title,
            declare_binary: false,
            mtime_ms: None,
        };
        self.own_stage(&key, &object.id)?;
        let _path = self.upload_path_guard(&key)?;
        let mime = self.copy_as(&source_id, &object, size)?;
        let objects = self.named_objects(&object.parent_id, &object.title)?;
        if objects.len() != 1 || objects[0].id != object.id {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "Der Drive-Stufenname führt nicht eindeutig zur eigenen ID {}",
                    object.id
                ),
            ));
        }
        self.remember_path(&key, &object.id, Some(&mime))?;
        self.persist_path_cache();
        Ok(Some(size))
    }
}

fn taken(path: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("Name in Google Drive bereits vergeben: /{path}"),
    )
}
