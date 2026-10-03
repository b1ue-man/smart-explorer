use super::api::{
    drive_request, err, mutation_once, parse_json, MutationRequestError, FOLDER_MIME,
};
use super::core::{cloud_urlenc, norm, parse_rfc3339_ms, split_parent};
use super::folder_create_journal::PendingFolderCreate;
use super::GDriveBackend;
use crate::vfs::{VfsMeta, VfsResult};
use std::io;

impl GDriveBackend {
    /// A named object with incomplete metadata stays a protected omission.
    /// Browsing retains its existing best-effort metadata representation.
    pub(super) fn sync_metadata_problem(f: &serde_json::Value) -> Option<&'static str> {
        let Some(mime) = f["mimeType"].as_str().filter(|mime| !mime.is_empty()) else {
            return Some("Drive object type is unavailable");
        };
        if f["modifiedTime"]
            .as_str()
            .and_then(parse_rfc3339_ms)
            .is_none()
        {
            return Some("Drive modification time is unavailable or invalid");
        }
        if !mime.starts_with("application/vnd.google-apps.")
            && f["size"]
                .as_str()
                .and_then(|size| size.parse::<u64>().ok())
                .is_none()
        {
            return Some("Drive binary size is unavailable or invalid");
        }
        None
    }
    /// The Drive mimeType for `path` (cached from list_dir, else a stat call).
    pub(super) fn mime_of(&self, path: &str) -> Option<String> {
        let key = norm(path);
        if let Some(m) = self.mimes_guard().ok()?.get(&key).cloned() {
            let trusted_id = self
                .cached_id(&key)
                .ok()
                .flatten()
                .is_some_and(|_| self.cached_id_is_trusted(&key).unwrap_or(false));
            if trusted_id {
                return Some(m);
            }
        }
        let _ = self.resolve(&key).ok()?;
        if let Some(m) = self.mimes_guard().ok()?.get(&key).cloned() {
            return Some(m);
        }
        let id = self.resolve(&key).ok()?;
        let url = self.api_url(&format!("files/{}?fields=mimeType", cloud_urlenc(&id)));
        let v = self.get_json(&url).ok()?;
        let m = v["mimeType"].as_str()?.to_string();
        self.mimes_guard().ok()?.insert(key, m.clone());
        Some(m)
    }

    pub(super) fn meta_from_json(
        f: &serde_json::Value,
        fallback_name: Option<&str>,
    ) -> Option<VfsMeta> {
        let is_dir = f["mimeType"].as_str() == Some(FOLDER_MIME);
        let mime = f["mimeType"].as_str().unwrap_or("");
        let is_symlink = mime == "application/vnd.google-apps.shortcut";
        let special = !is_dir && !is_symlink && mime.starts_with("application/vnd.google-apps.");
        let name = f["name"]
            .as_str()
            .or(fallback_name)
            .filter(|name| !name.is_empty())?;
        Some(VfsMeta {
            name: name.to_string(),
            is_dir,
            is_symlink,
            special,
            size: f["size"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0),
            mtime_ms: f["modifiedTime"]
                .as_str()
                .and_then(parse_rfc3339_ms)
                .unwrap_or(0),
            btime_ms: f["createdTime"]
                .as_str()
                .and_then(parse_rfc3339_ms)
                .unwrap_or(0),
            hidden: false,
            system: false,
            id: f["id"].as_str().map(|s| s.to_string()),
            content_md5: f["md5Checksum"]
                .as_str()
                .filter(|hash| {
                    hash.len() == 32 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                .map(str::to_string),
        })
    }

    /// MIME type of a file by its id (for export detection when opening by id).
    pub(super) fn mime_of_id(&self, id: &str) -> Option<String> {
        let url = self.api_url(&format!("files/{}?fields=mimeType", cloud_urlenc(id)));
        self.get_json(&url).ok()?["mimeType"]
            .as_str()
            .map(|s| s.to_string())
    }

    /// Ensure a folder path exists, returning the deepest folder's id.
    /// Thread-safe: concurrent transfers may need the same folder, so the
    /// find-or-create of each level holds that level's namespace slot (parent
    /// ID + encoded name); parents are resolved first, outside the slot, and
    /// other folders are created in parallel.
    pub(super) fn ensure_dir(&self, path: &str) -> VfsResult<String> {
        let key = norm(path);
        if key.is_empty() {
            return Ok("root".to_string());
        }
        let pending = self.pending_folder_create(&key)?;
        if pending.is_none() {
            if let Some(id) = self.valid_cached_id(&key)? {
                return Ok(id);
            }
        }
        let (parent, name) = split_parent(&key);
        let parent_id = self.ensure_dir(&parent)?;

        let _slot = self.create_slot_guard(&parent_id, name)?;
        if let Some(pending) = self.pending_folder_create(&key)? {
            return self.resume_pending_folder_create(&parent, &key, name, &parent_id, &pending);
        }
        // Re-check in the slot - another thread may have just created it.
        if let Some(id) = self.valid_cached_id(&key)? {
            return Ok(id);
        }
        // If the parent's children are fully known and this folder isn't among
        // them, it's known-absent -> skip the existence query.
        let known_absent = self.listed_guard()?.contains(&parent);
        let existing = if known_absent {
            None
        } else {
            self.find_child(&parent_id, name)?
        };
        if let Some(id) = existing {
            self.remember_path(&key, &id, None)?;
            self.persist_path_cache();
            return Ok(id);
        }
        // New folder metadata uses the original Drive title.
        let decoded_name = super::names::decode(name)?;
        let title = decoded_name.as_str();
        // Create the folder.
        let reserved_id = self.take_generated_id()?;
        let (pending, newly_claimed) =
            self.reserve_pending_folder_create(&key, &reserved_id, title, &parent_id)?;
        if !newly_claimed {
            return self.resume_pending_folder_create(&parent, &key, name, &parent_id, &pending);
        }
        self.submit_reserved_folder(&parent, &key, &pending, None)
    }

    pub(super) fn resume_pending_folder_create(
        &self,
        parent: &str,
        key: &str,
        name: &str,
        parent_id: &str,
        pending: &PendingFolderCreate,
    ) -> VfsResult<String> {
        if pending.key != key
            || pending.name != super::names::decode(name)?
            || pending.parent_id != parent_id
            || pending.account_key.as_str() != self.drive_account_key.as_ref()
        {
            return Err(io::Error::other(format!(
                "Drive pending folder reservation {} no longer matches path {key} and its parent",
                pending.id
            )));
        }
        match self.verify_created_folder(&pending.id, &pending.name, &pending.parent_id) {
            Ok(()) => self.finish_reserved_folder(key, pending),
            Err(preflight_error) => {
                self.submit_reserved_folder(parent, key, pending, Some(preflight_error))
            }
        }
    }

    pub(super) fn submit_reserved_folder(
        &self,
        parent: &str,
        key: &str,
        pending: &PendingFolderCreate,
        preflight_error: Option<io::Error>,
    ) -> VfsResult<String> {
        let body = serde_json::json!({
            "id": pending.id.clone(),
            "name": pending.name.clone(),
            "mimeType": FOLDER_MIME,
            "parents": [pending.parent_id.clone()],
        });
        let auth = self.bearer()?;
        let bearer = format!("Bearer {}", auth);
        let payload = body.to_string();
        let create_url = self.api_url("files?fields=id");
        let response = mutation_once(drive_request(
            self.timed_request(self.http.api().post(&create_url))
                .set("Authorization", &bearer)
                .set("Content-Type", "application/json")
                .send_string(&payload),
        ));
        match response {
            Ok(response) => {
                let response_state = response
                    .into_string()
                    .map_err(err)
                    .and_then(parse_json)
                    .and_then(|json| {
                        if json["id"].as_str() == Some(&pending.id) {
                            Ok(())
                        } else {
                            Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "Drive folder create returned an unexpected object ID",
                            ))
                        }
                    });
                if let Err(response_error) = response_state {
                    return self.verify_reserved_folder_after_error(
                        parent,
                        key,
                        pending,
                        combine_attempt_errors(preflight_error, response_error),
                    );
                }
            }
            Err(MutationRequestError::Definite(error))
                if crate::vfs::congestion_of(&error).is_some() =>
            {
                // Drive refused the create before running it. The durable
                // reservation stays, so the retry after slowing down sends the
                // same ID again (409 if an earlier attempt did commit).
                return Err(error);
            }
            Err(MutationRequestError::Definite(error))
            | Err(MutationRequestError::Ambiguous(error)) => {
                return self.verify_reserved_folder_after_error(
                    parent,
                    key,
                    pending,
                    combine_attempt_errors(preflight_error, error),
                );
            }
        }
        self.finish_reserved_folder(key, pending)
    }

    fn verify_reserved_folder_after_error(
        &self,
        parent: &str,
        key: &str,
        pending: &PendingFolderCreate,
        operation_error: io::Error,
    ) -> VfsResult<String> {
        match self.verify_created_folder(&pending.id, &pending.name, &pending.parent_id) {
            Ok(()) => self.finish_reserved_folder(key, pending),
            Err(verify_error) => {
                self.invalidate_folder_create(parent, key);
                Err(ambiguous_create(
                    &pending.id,
                    &operation_error,
                    &verify_error,
                ))
            }
        }
    }

    fn finish_reserved_folder(
        &self,
        key: &str,
        pending: &PendingFolderCreate,
    ) -> VfsResult<String> {
        self.remember_path(key, &pending.id, Some(FOLDER_MIME))?;
        // A brand-new folder has no children -> its contents are fully known.
        self.listed_guard()?.insert(key.to_string());
        // Keep the durable reservation until the ordinary path cache is safely
        // written. A failed cache write therefore remains retryable by exact ID.
        self.persist_path_cache_checked()?;
        self.clear_pending_folder_create(pending)?;
        Ok(pending.id.clone())
    }

    fn verify_created_folder(&self, id: &str, name: &str, parent_id: &str) -> VfsResult<()> {
        let url = self.api_url(&format!(
            "files/{}?fields=id,name,parents,mimeType,trashed",
            cloud_urlenc(id)
        ));
        let json = self.get_json(&url)?;
        if folder_state_matches(&json, id, name, parent_id) {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Drive folder ID does not have the expected final state",
            ))
        }
    }

    fn invalidate_folder_create(&self, parent: &str, key: &str) {
        if let Ok(mut listed) = self.listed_guard() {
            listed.remove(parent);
        }
        self.forget_path_prefix(key);
    }
}

fn folder_state_matches(json: &serde_json::Value, id: &str, name: &str, parent_id: &str) -> bool {
    json["id"].as_str() == Some(id)
        && json["name"].as_str() == Some(name)
        && json["mimeType"].as_str() == Some(FOLDER_MIME)
        && json["trashed"].as_bool() == Some(false)
        && json["parents"]
            .as_array()
            .is_some_and(|parents| parents.len() == 1 && parents[0].as_str() == Some(parent_id))
}

fn ambiguous_create(id: &str, send: &io::Error, verify: &io::Error) -> io::Error {
    io::Error::other(format!(
        "Drive folder create for exact ID {id} has ambiguous completion: request response was unavailable or invalid ({send}); exact-ID final-state verification failed ({verify})"
    ))
}

fn combine_attempt_errors(preflight: Option<io::Error>, attempt: io::Error) -> io::Error {
    let Some(preflight) = preflight else {
        return attempt;
    };
    io::Error::new(
        attempt.kind(),
        format!(
            "exact-ID verification before the same-ID create retry failed ({preflight}); same-ID create attempt failed ({attempt})"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn meta_from_json_requires_a_usable_name() {
        let f = json!({"id": "1", "mimeType": "text/plain"});
        assert!(GDriveBackend::meta_from_json(&f, None).is_none());

        let m = GDriveBackend::meta_from_json(&f, Some("fallback.txt")).unwrap();
        assert_eq!(m.name, "fallback.txt");
    }
}
