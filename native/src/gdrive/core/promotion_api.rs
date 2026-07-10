use super::api::{drive_request, parse_json, send_retry, API};
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
            let mut url = format!(
                "{API}/files?q={}&fields=nextPageToken,files(id,name,mimeType,size,md5Checksum,parents,trashed)&pageSize={QUERY_LIMIT}",
                cloud_urlenc(&query)
            );
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
        let mut url = format!("{API}/files/{}?fields=id", cloud_urlenc(id));
        if source_parent_id != destination_parent_id {
            url.push_str(&format!(
                "&addParents={}&removeParents={}",
                cloud_urlenc(destination_parent_id),
                cloud_urlenc(source_parent_id)
            ));
        }
        let bearer = format!("Bearer {}", self.bearer()?);
        let payload = serde_json::json!({ "name": destination_name }).to_string();
        let response = parse_json(send_retry(|| {
            drive_request(
                ureq::request("PATCH", &url)
                    .set("Authorization", &bearer)
                    .set("Content-Type", "application/json")
                    .send_string(&payload),
            )
        })?)?;
        if response["id"].as_str() != Some(id) {
            return Err(invalid("Drive rename returned an unexpected object ID"));
        }
        Ok(())
    }
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
