use super::api::{drive_request, mutation_once, parse_json, MutationRequestError};
use super::core::cloud_urlenc;
use super::GDriveBackend;
use crate::vfs::VfsResult;
use std::collections::HashSet;
use std::io;

const QUERY_LIMIT: usize = 2;
const MAX_QUERY_PAGES: usize = 1_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DriveObject {
    pub(super) id: String,
    pub(super) mime_type: String,
    pub(super) size: Option<u64>,
    pub(super) md5: Option<String>,
}

impl GDriveBackend {
    /// Return at most two exact-name children: zero means absent, one is
    /// unambiguous, and two is sufficient to reject a duplicate namespace.
    pub(super) fn named_objects(&self, parent_id: &str, name: &str) -> VfsResult<Vec<DriveObject>> {
        let query = format!(
            "'{}' in parents and name = '{}' and trashed = false",
            query_literal(parent_id),
            query_literal(name)
        );
        let mut output = Vec::with_capacity(QUERY_LIMIT);
        let mut page_token: Option<String> = None;
        let mut seen_tokens = HashSet::new();
        for _ in 0..MAX_QUERY_PAGES {
            let mut url = self.api_url(&format!(
                "files?q={}&fields=nextPageToken,files(id,name,mimeType,size,md5Checksum,parents,trashed)&pageSize={QUERY_LIMIT}",
                cloud_urlenc(&query)
            ));
            if let Some(token) = page_token.as_deref() {
                url.push_str(&format!("&pageToken={}", cloud_urlenc(token)));
            }
            let json = self.get_json(&url)?;
            if let Some(files) = json["files"].as_array() {
                for file in files {
                    let object = parse_object(file, parent_id, name)?;
                    if output
                        .iter()
                        .any(|existing: &DriveObject| existing.id == object.id)
                    {
                        return Err(invalid("Drive returned the same object ID more than once"));
                    }
                    output.push(object);
                    if output.len() == QUERY_LIMIT {
                        return Ok(output);
                    }
                }
            }
            page_token = json["nextPageToken"]
                .as_str()
                .filter(|token| !token.is_empty())
                .map(str::to_owned);
            let Some(token) = page_token.as_ref() else {
                return Ok(output);
            };
            if !seen_tokens.insert(token.clone()) {
                return Err(invalid("Drive repeated a file-list page token"));
            }
        }
        Err(invalid("Drive named-object query exceeded its page budget"))
    }

    /// Change the name/parent of one exact Drive ID. The caller owns namespace
    /// locking and performs any required uniqueness verification.
    pub(super) fn rename_id(
        &self,
        id: &str,
        source_parent_id: &str,
        destination_parent_id: &str,
        destination_name: &str,
    ) -> VfsResult<()> {
        let mut url = self.api_url(&format!("files/{}?fields=id", cloud_urlenc(id)));
        if source_parent_id != destination_parent_id {
            url.push_str(&format!(
                "&addParents={}&removeParents={}",
                cloud_urlenc(destination_parent_id),
                cloud_urlenc(source_parent_id)
            ));
        }
        let bearer = format!("Bearer {}", self.bearer()?);
        let payload = serde_json::json!({ "name": destination_name }).to_string();
        let response = mutation_once(drive_request(
            self.timed_request(ureq::request("PATCH", &url))
                .set("Authorization", &bearer)
                .set("Content-Type", "application/json")
                .send_string(&payload),
        ));
        match response {
            Ok(response) => {
                let response_state = response
                    .into_string()
                    .map_err(super::api::err)
                    .and_then(parse_json)
                    .and_then(|json| {
                        if json["id"].as_str() == Some(id) {
                            Ok(())
                        } else {
                            Err(invalid("Drive rename returned an unexpected object ID"))
                        }
                    });
                if let Err(response_error) = response_state {
                    if let Err(verify_error) =
                        self.verify_renamed_id(id, destination_parent_id, destination_name)
                    {
                        return Err(ambiguous_rename(id, &response_error, &verify_error));
                    }
                }
            }
            Err(MutationRequestError::Definite(error)) => return Err(error),
            Err(MutationRequestError::Ambiguous(send_error)) => {
                if let Err(verify_error) =
                    self.verify_renamed_id(id, destination_parent_id, destination_name)
                {
                    return Err(ambiguous_rename(id, &send_error, &verify_error));
                }
            }
        }
        Ok(())
    }

    fn verify_renamed_id(
        &self,
        id: &str,
        destination_parent_id: &str,
        destination_name: &str,
    ) -> VfsResult<()> {
        let url = self.api_url(&format!(
            "files/{}?fields=id,name,parents,mimeType,trashed",
            cloud_urlenc(id)
        ));
        let json = self.get_json(&url)?;
        let expected = json["id"].as_str() == Some(id)
            && json["name"].as_str() == Some(destination_name)
            && json["trashed"].as_bool() == Some(false)
            && json["mimeType"]
                .as_str()
                .is_some_and(|mime| !mime.is_empty())
            && json["parents"].as_array().is_some_and(|parents| {
                parents.len() == 1 && parents[0].as_str() == Some(destination_parent_id)
            });
        if expected {
            Ok(())
        } else {
            Err(invalid(
                "Drive rename exact ID does not have the expected final name and parent",
            ))
        }
    }
}

fn ambiguous_rename(id: &str, send: &io::Error, verify: &io::Error) -> io::Error {
    io::Error::other(format!(
        "Drive rename for exact ID {id} has ambiguous completion: PATCH response was unavailable or invalid ({send}); exact-ID final-state verification failed ({verify})"
    ))
}

fn parse_object(
    json: &serde_json::Value,
    expected_parent_id: &str,
    expected_name: &str,
) -> VfsResult<DriveObject> {
    if json["name"].as_str() != Some(expected_name)
        || json["trashed"].as_bool() != Some(false)
        || !json["parents"].as_array().is_some_and(|parents| {
            parents
                .iter()
                .any(|parent| parent.as_str() == Some(expected_parent_id))
        })
    {
        return Err(invalid(
            "Drive named-object query returned mismatched metadata",
        ));
    }
    let id = required_text(json, "id")?;
    let mime_type = required_text(json, "mimeType")?;
    let size = json["size"]
        .as_str()
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| invalid("Drive object has an invalid size"))
        })
        .transpose()?;
    let md5 = json["md5Checksum"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    Ok(DriveObject {
        id,
        mime_type,
        size,
        md5,
    })
}

fn required_text(json: &serde_json::Value, field: &str) -> VfsResult<String> {
    json[field]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid(format!("Drive object has no usable {field}")))
}

fn query_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_literals_escape_backslashes_and_quotes() {
        assert_eq!(query_literal("a\\b'c"), r"a\\b\'c");
    }

    #[test]
    fn object_parser_rejects_wrong_parent() {
        let json = serde_json::json!({
            "id": "id",
            "name": "file",
            "mimeType": "application/octet-stream",
            "parents": ["other"],
            "trashed": false
        });
        assert!(parse_object(&json, "parent", "file").is_err());
    }
}
