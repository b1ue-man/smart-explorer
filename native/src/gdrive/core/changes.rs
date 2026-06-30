use super::api::API;
use super::core::cloud_urlenc;
use super::GDriveBackend;
use crate::vfs::{ChangeKind, VfsChange, VfsChangeBatch, VfsResult};

const CHANGE_FIELDS: &str = "nextPageToken,newStartPageToken,changes(fileId,removed,time,file(id,name,parents,size,md5Checksum,modifiedTime,createdTime,mimeType,trashed))";

impl GDriveBackend {
    pub(super) fn start_page_token(&self) -> VfsResult<String> {
        let url = format!("{}/changes/startPageToken?fields=startPageToken", API);
        let v = self.get_json(&url)?;
        v["startPageToken"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "Drive-Token fehlt")
            })
    }

    pub(super) fn drive_changes_since(&self, cursor: &str) -> VfsResult<VfsChangeBatch> {
        let mut page = cursor.to_string();
        let mut all = VfsChangeBatch::default();
        loop {
            let url = format!(
                "{}/changes?pageToken={}&pageSize=1000&includeRemoved=true&spaces=drive&fields={}",
                API,
                cloud_urlenc(&page),
                cloud_urlenc(CHANGE_FIELDS)
            );
            let v = match self.get_json(&url) {
                Ok(v) => v,
                Err(e) if e.to_string().contains("HTTP 410") => {
                    return Ok(VfsChangeBatch {
                        reset: true,
                        ..Default::default()
                    })
                }
                Err(e) => return Err(e),
            };
            let mut batch = parse_changes_value(&v);
            all.changes.append(&mut batch.changes);
            if let Some(next) = v["nextPageToken"].as_str() {
                page = next.to_string();
                continue;
            }
            all.new_cursor = v["newStartPageToken"].as_str().map(|s| s.to_string());
            return Ok(all);
        }
    }
}

pub(super) fn parse_changes_value(v: &serde_json::Value) -> VfsChangeBatch {
    let mut out = VfsChangeBatch {
        new_cursor: v["newStartPageToken"].as_str().map(|s| s.to_string()),
        ..Default::default()
    };
    if let Some(changes) = v["changes"].as_array() {
        for ch in changes {
            let file = &ch["file"];
            let removed = ch["removed"].as_bool().unwrap_or(false)
                || file["trashed"].as_bool().unwrap_or(false);
            let id = ch["fileId"]
                .as_str()
                .or_else(|| file["id"].as_str())
                .map(|s| s.to_string());
            let parent_id = file["parents"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|p| p.as_str())
                .map(|s| s.to_string());
            let name = file["name"].as_str().map(|s| s.to_string());
            let meta = (!removed)
                .then(|| GDriveBackend::meta_from_json(file, name.as_deref()))
                .flatten();
            out.changes.push(VfsChange {
                kind: if removed {
                    ChangeKind::Remove
                } else {
                    ChangeKind::Upsert
                },
                rel: None,
                id,
                parent_id,
                name,
                meta,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_upsert_and_removed_changes() {
        let v: serde_json::Value = serde_json::json!({
            "newStartPageToken": "tok2",
            "changes": [
                {
                    "fileId": "id-a",
                    "removed": false,
                    "file": {
                        "id": "id-a",
                        "name": "a.txt",
                        "parents": ["root"],
                        "size": "12",
                        "md5Checksum": "900150983cd24fb0d6963f7d28e17f72",
                        "modifiedTime": "2024-06-01T12:34:56Z",
                        "mimeType": "text/plain",
                        "trashed": false
                    }
                },
                {
                    "fileId": "id-b",
                    "removed": true
                }
            ]
        });
        let b = parse_changes_value(&v);
        assert_eq!(b.new_cursor.as_deref(), Some("tok2"));
        assert_eq!(b.changes.len(), 2);
        assert_eq!(b.changes[0].kind, ChangeKind::Upsert);
        assert_eq!(b.changes[0].parent_id.as_deref(), Some("root"));
        assert_eq!(b.changes[0].meta.as_ref().unwrap().size, 12);
        assert_eq!(b.changes[1].kind, ChangeKind::Remove);
        assert_eq!(b.changes[1].id.as_deref(), Some("id-b"));
    }

    #[test]
    fn trashed_file_is_a_remove() {
        let v: serde_json::Value = serde_json::json!({
            "changes": [{
                "fileId": "id-a",
                "file": {"id": "id-a", "name": "a.txt", "trashed": true}
            }]
        });
        let b = parse_changes_value(&v);
        assert_eq!(b.changes[0].kind, ChangeKind::Remove);
        assert!(b.changes[0].meta.is_none());
    }
}
