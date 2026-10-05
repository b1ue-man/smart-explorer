//! Creating a new binary object under a pre-generated ID: one multipart
//! request for a small file (Google: "a small file (5 MB or less)", ref §1),
//! a resumable session for a larger one, a server-side copy for Drive-internal
//! copies. Every outcome is settled by the exact ID, never by a replay.
use super::api::{drive_request, err, mutation_once, parse_json, MutationRequestError};
use super::core::cloud_urlenc;
use super::overload::{http_status, random_token};
use super::resumable::{Resumable, REQUEST_TIMEOUT};
use super::GDriveBackend;
use crate::vfs::VfsResult;
use std::io::{self, Read};

/// Fields of a created object that prove the upload: identity, place, content.
pub(super) const CREATED_FIELDS: &str = "id,name,parents,size,md5Checksum,mimeType,trashed";
/// Declared type of uploaded bytes, as for every other upload of this backend.
pub(super) const MEDIA_TYPE: &str = "application/octet-stream";

/// A new object: its reserved ID, its place and title.
pub(super) struct NewObject {
    pub(super) id: String,
    pub(super) parent_id: String,
    pub(super) title: String,
    /// Copy stages declare a binary type (promotion requires one), like the
    /// spooled copy stage; new files leave the type to Drive like `open_write`.
    pub(super) declare_binary: bool,
    pub(super) mtime_ms: Option<i64>,
}

impl NewObject {
    pub(super) fn metadata(&self) -> String {
        let mut metadata = serde_json::json!({
            "id": self.id,
            "name": self.title,
            "parents": [self.parent_id],
        });
        if self.declare_binary {
            metadata["mimeType"] = serde_json::Value::from(MEDIA_TYPE);
        }
        if let Some(time) = self.mtime_ms.and_then(super::stage_time::formatted) {
            metadata["modifiedTime"] = serde_json::Value::from(time);
        }
        metadata.to_string()
    }
}

impl GDriveBackend {
    /// Create `object` from exactly `size` bytes of `content` in one
    /// `multipart/related` request (RFC 2387: metadata part, media part).
    /// Returns the MIME type once ID, place, size and MD5 are confirmed.
    pub(super) fn create_multipart(
        &self,
        object: &NewObject,
        content: impl Read,
        size: u64,
        md5: &str,
    ) -> VfsResult<String> {
        let boundary = format!("se-{}", random_token()?);
        let head = format!(
            "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{}\r\n--{boundary}\r\nContent-Type: {MEDIA_TYPE}\r\n\r\n",
            object.metadata()
        );
        let tail = format!("\r\n--{boundary}--\r\n");
        let length = head.len() as u64 + size + tail.len() as u64;
        let body = io::Cursor::new(head.into_bytes())
            .chain(content.take(size))
            .chain(io::Cursor::new(tail.into_bytes()));
        let url = format!(
            "{}?uploadType=multipart&fields={CREATED_FIELDS}",
            self.upload_url()
        );
        let bearer = format!("Bearer {}", self.bearer()?);
        // An explicit length keeps ureq from chunking the streamed body.
        let result = mutation_once(drive_request(
            self.http
                .api()
                .post(&url)
                .timeout(REQUEST_TIMEOUT)
                .set("Authorization", &bearer)
                .set(
                    "Content-Type",
                    &format!("multipart/related; boundary={boundary}"),
                )
                .set("Content-Length", &length.to_string())
                .send(body),
        ));
        self.settle_create(object, result, size, Some(md5))
    }

    /// Open a resumable session that creates `object` when its last byte
    /// arrives; until then nothing exists under the name.
    pub(super) fn start_new_upload(&self, object: &NewObject, size: u64) -> VfsResult<Resumable> {
        let url = format!(
            "{}?uploadType=resumable&fields={CREATED_FIELDS}",
            self.upload_url()
        );
        let bearer = format!("Bearer {}", self.bearer()?);
        let agent = self.http.api();
        let location =
            super::transfer::initiate(&agent, "POST", &url, &bearer, size, &object.metadata())?;
        Resumable::new(agent, &location, size, &object.id)
    }

    /// Server-side copy of `source_id` as `object` (`files.copy` accepts the
    /// pre-generated ID like `files.create`, ref §5).
    pub(super) fn copy_as(
        &self,
        source_id: &str,
        object: &NewObject,
        size: u64,
    ) -> VfsResult<String> {
        let url = self.api_url(&format!(
            "files/{}/copy?fields={CREATED_FIELDS}",
            cloud_urlenc(source_id)
        ));
        let bearer = format!("Bearer {}", self.bearer()?);
        let result = mutation_once(drive_request(
            self.http
                .api()
                .post(&url)
                .timeout(REQUEST_TIMEOUT)
                .set("Authorization", &bearer)
                .set("Content-Type", "application/json")
                .send_string(&object.metadata()),
        ));
        self.settle_create(object, result, size, None)
    }

    /// Decide a create by its exact reserved ID; never sends it again.
    pub(super) fn settle_create(
        &self,
        object: &NewObject,
        result: Result<ureq::Response, MutationRequestError>,
        size: u64,
        md5: Option<&str>,
    ) -> VfsResult<String> {
        let parent_id = self.actual_parent_id(&object.parent_id)?;
        let failure = match result {
            Ok(response) => match response.into_string().map_err(err).and_then(parse_json) {
                Ok(json) if created_matches(&json, object, &parent_id, size, md5) => {
                    return Ok(mime_of(&json))
                }
                Ok(_) => io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Drive bestätigte das neue Objekt mit abweichenden Angaben",
                ),
                Err(error) => error,
            },
            // Refused before running (rate limit as congestion, invalid
            // request): nothing was created, the ID stays unused.
            Err(MutationRequestError::Definite(error)) if http_status(&error) != Some(409) => {
                return Err(error)
            }
            // 409: the ID exists (an earlier attempt committed); 5xx and lost
            // answers may have committed. The exact ID decides.
            Err(MutationRequestError::Definite(error))
            | Err(MutationRequestError::Ambiguous(error)) => error,
        };
        self.verify_created(object, size, md5, failure)
    }

    /// Look the reserved ID up: the expected state proves the create, absence
    /// proves nothing was created (then `failure` is returned unchanged, so a
    /// rate limit stays congestion for the caller's retry of the same ID).
    pub(super) fn verify_created(
        &self,
        object: &NewObject,
        size: u64,
        md5: Option<&str>,
        failure: io::Error,
    ) -> VfsResult<String> {
        let parent_id = self.actual_parent_id(&object.parent_id)?;
        let url = self.api_url(&format!(
            "files/{}?fields={CREATED_FIELDS}",
            cloud_urlenc(&object.id)
        ));
        match self.get_json(&url) {
            Ok(json) if created_matches(&json, object, &parent_id, size, md5) => Ok(mime_of(&json)),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Drive-Objekt {} hat nach dem Hochladen nicht den erwarteten Namen, Ort, Umfang oder Inhalt ({failure})",
                    object.id
                ),
            )),
            Err(lookup) if http_status(&lookup) == Some(404) => Err(failure),
            Err(lookup) => Err(io::Error::new(
                failure.kind(),
                format!(
                    "Ergebnis des Drive-Uploads für ID {} ist unklar ({failure}); die Prüfung per ID scheiterte: {lookup}",
                    object.id
                ),
            )),
        }
    }
}

/// The object shows exactly the reserved ID, title, single parent, size and
/// MD5 (the given one, else any), is not trashed and holds binary content.
fn created_matches(
    json: &serde_json::Value,
    object: &NewObject,
    parent_id: &str,
    size: u64,
    md5: Option<&str>,
) -> bool {
    let mime = json["mimeType"].as_str().unwrap_or("");
    json["id"].as_str() == Some(object.id.as_str())
        && json["name"].as_str() == Some(object.title.as_str())
        && json["trashed"].as_bool() == Some(false)
        && json["parents"]
            .as_array()
            .is_some_and(|parents| parents.len() == 1 && parents[0].as_str() == Some(parent_id))
        && json["size"]
            .as_str()
            .and_then(|value| value.parse::<u64>().ok())
            == Some(size)
        && match json["md5Checksum"].as_str() {
            Some(actual) => md5.map_or(!actual.is_empty(), |md5| actual.eq_ignore_ascii_case(md5)),
            // An empty file has nothing to compare if Drive omits its checksum.
            None => size == 0,
        }
        && !mime.is_empty()
        && !mime.starts_with("application/vnd.google-apps.")
}

fn mime_of(json: &serde_json::Value) -> String {
    json["mimeType"].as_str().unwrap_or(MEDIA_TYPE).to_string()
}
