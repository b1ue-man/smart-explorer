//! Drive's root alias and fresh, typed object-identity evidence.
use super::api::FOLDER_MIME;
use super::core::cloud_urlenc;
use super::overload::http_status;
use super::GDriveBackend;
use serde_json::Value;
use std::io;

pub(super) const OBJECT_FIELDS: &str =
    "id,name,mimeType,parents,trashed,size,md5Checksum,modifiedTime,createdTime";

impl GDriveBackend {
    /// `parents` contains the actual root ID even when a request used `root`.
    /// Do not hold the mutex across a network call.
    pub(super) fn actual_parent_id(&self, id: &str) -> io::Result<String> {
        if id != "root" {
            return Ok(id.to_string());
        }
        if let Some(id) = self.root_id_guard()?.clone() {
            return Ok(id);
        }
        let json = self.get_json(&self.api_url("files/root?fields=id"))?;
        let id = text(&json, "id")?.to_string();
        let mut root = self.root_id_guard()?;
        if root.as_ref().is_some_and(|previous| previous != &id) {
            return Err(invalid("Drive root identity changed within one connection"));
        }
        *root = Some(id.clone());
        Ok(id)
    }

    pub(super) fn object_json(&self, id: &str) -> io::Result<Value> {
        let json = self.get_json(&self.api_url(&format!(
            "files/{}?fields={OBJECT_FIELDS}",
            cloud_urlenc(id)
        )))?;
        if text(&json, "id")? != id {
            return Err(invalid("Drive returned a different selected object ID"));
        }
        Ok(json)
    }

    /// Missing is returned only for fresh HTTP 404 or complete contradictory
    /// identity metadata. Permission/transport/parse failures remain errors.
    pub(super) fn folder_evidence(
        &self,
        parent: &str,
        title: &str,
        id: &str,
    ) -> io::Result<Option<Value>> {
        let json = match self.object_json(id) {
            Ok(json) => json,
            Err(error) if http_status(&error) == Some(404) => return Ok(None),
            Err(error) => return Err(error),
        };
        if !in_parent(&json, parent)?
            || text(&json, "name")? != title
            || text(&json, "mimeType")? != FOLDER_MIME
        {
            return Ok(None);
        }
        Ok(Some(json))
    }

    pub(super) fn narrowed_listing_corpus(&self, parent: &str) -> io::Result<String> {
        let json = self.get_json(
            &self.api_url(&format!("files/{}?fields=id,driveId", cloud_urlenc(parent))),
        )?;
        if text(&json, "id")? != parent {
            return Err(invalid("Drive returned a different listing parent"));
        }
        match json.get("driveId") {
            None | Some(Value::Null) => Ok("&corpora=user".to_string()),
            Some(Value::String(id)) if !id.is_empty() => {
                Ok(format!("&corpora=drive&driveId={}", cloud_urlenc(id)))
            }
            _ => Err(invalid("Drive returned an invalid drive corpus ID")),
        }
    }
}

pub(super) fn text<'a>(json: &'a Value, field: &str) -> io::Result<&'a str> {
    json[field]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(format!("Drive object has no usable {field}")))
}

pub(super) fn in_parent(json: &Value, parent: &str) -> io::Result<bool> {
    let trashed = json["trashed"]
        .as_bool()
        .ok_or_else(|| invalid("Drive object has no trash-state evidence"))?;
    if trashed {
        return Ok(false);
    }
    let parents = json["parents"]
        .as_array()
        .ok_or_else(|| invalid("Drive object has no parent-identity evidence"))?;
    if parents
        .iter()
        .any(|parent| parent.as_str().is_none_or(str::is_empty))
    {
        return Err(invalid("Drive object has invalid parent identities"));
    }
    Ok(parents.len() == 1 && parents[0].as_str() == Some(parent))
}

pub(super) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
