use super::api::{drive_request, mutation_once, parse_json, MutationRequestError};
use super::core::{cloud_urlenc, norm, split_parent};
use super::GDriveBackend;
use crate::vfs::VfsResult;
use std::io;

enum TrashFailure {
    Definite(io::Error),
    Ambiguous(io::Error),
}

impl TrashFailure {
    fn into_io(self) -> io::Error {
        match self {
            Self::Definite(error) | Self::Ambiguous(error) => error,
        }
    }
}

impl GDriveBackend {
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
        match self.trash_id_once(&id) {
            Ok(()) => {
                if pending_id.is_some() {
                    self.pending_upload_ids_guard()?.remove(&key);
                }
                self.forget_path_prefix(&key);
                Ok(())
            }
            Err(TrashFailure::Definite(error)) => Err(error),
            Err(TrashFailure::Ambiguous(error)) => {
                self.invalidate_ambiguous_trash(&key);
                Err(error)
            }
        }
    }

    pub(super) fn trash_path_id(&self, path: &str, id: &str) -> VfsResult<()> {
        let key = norm(path);
        let _path_guard = self.upload_path_guard(&key)?;
        match self.trash_id_once(id) {
            Ok(()) => {
                self.forget_path_prefix(&key);
                Ok(())
            }
            Err(TrashFailure::Definite(error)) => Err(error),
            Err(TrashFailure::Ambiguous(error)) => {
                self.invalidate_ambiguous_trash(&key);
                Err(error)
            }
        }
    }

    /// Trash one file by its exact id (targets a specific duplicate-named file).
    pub(super) fn trash_id(&self, id: &str) -> VfsResult<()> {
        self.trash_id_once(id).map_err(TrashFailure::into_io)
    }

    fn trash_id_once(&self, id: &str) -> Result<(), TrashFailure> {
        let auth = self.bearer().map_err(TrashFailure::Definite)?;
        let bearer = format!("Bearer {auth}");
        let url = self.api_url(&format!("files/{}?fields=id,trashed", cloud_urlenc(id)));
        let payload = serde_json::json!({ "trashed": true }).to_string();
        match mutation_once(drive_request(
            self.timed_request(ureq::request("PATCH", &url))
                .set("Authorization", &bearer)
                .set("Content-Type", "application/json")
                .send_string(&payload),
        )) {
            Ok(response) => {
                let response_state = response
                    .into_string()
                    .map_err(super::api::err)
                    .and_then(parse_json)
                    .and_then(|json| {
                        if trash_state_matches(&json, id) {
                            Ok(())
                        } else {
                            Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "Drive trash returned an unexpected exact-ID state",
                            ))
                        }
                    });
                match response_state {
                    Ok(()) => Ok(()),
                    Err(response_error) => match self.verify_trashed_id(id) {
                        Ok(()) => Ok(()),
                        Err(verify_error) => Err(TrashFailure::Ambiguous(ambiguous_trash(
                            id,
                            &response_error,
                            &verify_error,
                        ))),
                    },
                }
            }
            Err(MutationRequestError::Definite(error)) => Err(TrashFailure::Definite(error)),
            Err(MutationRequestError::Ambiguous(send_error)) => match self.verify_trashed_id(id) {
                Ok(()) => Ok(()),
                Err(verify_error) => Err(TrashFailure::Ambiguous(ambiguous_trash(
                    id,
                    &send_error,
                    &verify_error,
                ))),
            },
        }
    }

    fn verify_trashed_id(&self, id: &str) -> VfsResult<()> {
        let url = self.api_url(&format!("files/{}?fields=id,trashed", cloud_urlenc(id)));
        let json = self.get_json(&url)?;
        if trash_state_matches(&json, id) {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Drive exact ID is not confirmed trashed",
            ))
        }
    }

    fn invalidate_ambiguous_trash(&self, key: &str) {
        self.forget_path_prefix(key);
        let (parent, _) = split_parent(key);
        if let Ok(mut listed) = self.listed_guard() {
            listed.remove(&parent);
        }
        self.persist_path_cache();
    }
}

fn trash_state_matches(json: &serde_json::Value, id: &str) -> bool {
    json["id"].as_str() == Some(id) && json["trashed"].as_bool() == Some(true)
}

fn ambiguous_trash(id: &str, send: &io::Error, verify: &io::Error) -> io::Error {
    io::Error::other(format!(
        "Drive trash for exact ID {id} has ambiguous completion: PATCH response was unavailable or invalid ({send}); exact-ID trashed-state verification failed ({verify})"
    ))
}
