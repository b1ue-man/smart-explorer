//! Effective modifiedTime on exact Drive IDs, including media-update retries.
use super::api::{drive_request, mutation_once, MutationRequestError};
use super::core::{cloud_urlenc, parse_rfc3339_ms};
use super::GDriveBackend;
use crate::vfs::VfsResult;
use std::io;

pub(super) fn formatted(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

impl GDriveBackend {
    fn modified_time(&self, id: &str) -> VfsResult<Option<i64>> {
        let json = self.get_json(&self.api_url(&format!(
            "files/{}?fields=id,modifiedTime,trashed", cloud_urlenc(id))))?;
        if json["id"].as_str() != Some(id) || json["trashed"].as_bool() != Some(false) {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                "Drive modifiedTime lookup did not return the live selected ID"));
        }
        Ok(json["modifiedTime"].as_str().and_then(parse_rfc3339_ms))
    }

    pub(super) fn set_modified_time(&self, id: &str, ms: i64) -> VfsResult<bool> {
        let Some(time) = formatted(ms) else { return Ok(false) };
        if self.modified_time(id)? == Some(ms) { return Ok(true) }
        let url = self.api_url(&format!("files/{}?fields=id,modifiedTime", cloud_urlenc(id)));
        let payload = serde_json::json!({"modifiedTime": time}).to_string();
        let result = mutation_once(drive_request(self.timed_request(self.http.api().request("PATCH", &url))
            .set("Authorization", &format!("Bearer {}", self.bearer()?))
            .set("Content-Type", "application/json").send_string(&payload)));
        let failure = match result {
            Ok(_) => None,
            Err(MutationRequestError::Definite(error))
                if matches!(super::overload::http_status(&error), Some(400 | 403 | 405 | 501))
                    && crate::vfs::congestion_of(&error).is_none() => return Ok(false),
            Err(MutationRequestError::Definite(error) | MutationRequestError::Ambiguous(error)) => Some(error),
        };
        match self.modified_time(id) {
            Ok(actual) if actual == Some(ms) => Ok(true),
            Ok(_) => match failure { Some(error) => Err(error), None => Ok(false) },
            Err(error) => Err(error),
        }
    }

    pub(super) fn preserve_modified_time(&self, id: &str, ms: Option<i64>) -> VfsResult<()> {
        if let Some(ms) = ms {
            if !self.set_modified_time(id, ms)? {
                return Err(io::Error::new(io::ErrorKind::Unsupported,
                    "Drive publication did not preserve the verified stage time; keep the stage for retry"));
            }
        }
        Ok(())
    }
}
