use super::core::cloud_urlenc;
use super::GDriveBackend;
use crate::vfs::{ChangeKind, VfsChange, VfsChangeBatch, VfsResult};
use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

const CHANGE_FIELDS: &str = "nextPageToken,newStartPageToken,changes(fileId,removed,time,file(id,name,parents,size,md5Checksum,modifiedTime,createdTime,mimeType,trashed))";

impl GDriveBackend {
    pub(super) fn start_page_token(&self) -> VfsResult<String> {
        let url = self.api_url("changes/startPageToken?fields=startPageToken");
        let v = self.get_json(&url)?;
        v["startPageToken"]
            .as_str()
            .filter(|token| !token.is_empty())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "Drive-Token fehlt")
            })
    }

    pub(super) fn drive_changes_since(&self, cursor: &str) -> VfsResult<VfsChangeBatch> {
        self.read_changes(cursor, || false)
    }

    pub(super) fn drive_changes_since_poll(
        &self,
        cursor: &str,
        canceled: &AtomicBool,
    ) -> VfsResult<VfsChangeBatch> {
        self.read_changes(cursor, || canceled.load(Ordering::Acquire))
    }

    fn read_changes(&self, cursor: &str, canceled: impl Fn() -> bool) -> VfsResult<VfsChangeBatch> {
        if cursor.is_empty() {
            return Err(invalid("Drive change cursor is empty"));
        }
        let mut page = cursor.to_string();
        let mut all = VfsChangeBatch::default();
        let mut seen = HashSet::from([page.clone()]);
        let mut bytes = 0u64;
        let byte_limit = crate::transfer::memory_budget() / 8;
        let Some(_memory) = crate::transfer::try_reserve_memory(byte_limit) else {
            return Ok(VfsChangeBatch {
                reset: true,
                ..Default::default()
            });
        };
        for _ in 0..1_000 {
            if canceled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Drive feed poll canceled",
                ));
            }
            let url = self.api_url(&format!(
                "changes?pageToken={}&pageSize=1000&includeRemoved=true&spaces=drive&fields={}",
                cloud_urlenc(&page),
                cloud_urlenc(CHANGE_FIELDS)
            ));
            let v = match self.get_json(&url) {
                Ok(v) => v,
                Err(e) if super::overload::http_status(&e) == Some(410) => {
                    return Ok(VfsChangeBatch {
                        reset: true,
                        ..Default::default()
                    })
                }
                Err(e) => return Err(e),
            };
            if canceled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Drive feed poll canceled",
                ));
            }
            let mut batch = checked_changes(&v)?;
            for change in &batch.changes {
                // Bound the accumulated feed, including metadata and string
                // allocation overhead. A full scan can recover an overflow;
                // a partial feed must never advance the index cursor.
                bytes = bytes.saturating_add(
                    1024 + change.id.as_ref().map_or(0, |s| s.len() as u64)
                        + change.name.as_ref().map_or(0, |s| s.len() as u64)
                        + change.parent_id.as_ref().map_or(0, |s| s.len() as u64),
                );
                if bytes > byte_limit {
                    return Ok(VfsChangeBatch {
                        reset: true,
                        ..Default::default()
                    });
                }
            }
            all.changes.append(&mut batch.changes);
            if let Some(next) = token(&v, "nextPageToken")? {
                if token(&v, "newStartPageToken")?.is_some() || !seen.insert(next.to_string()) {
                    return Err(invalid("Drive repeated or combined feed page tokens"));
                }
                page = next.to_string();
                continue;
            }
            all.new_cursor = Some(
                token(&v, "newStartPageToken")?
                    .ok_or_else(|| invalid("Drive terminal change page has no new cursor"))?
                    .to_string(),
            );
            return Ok(all);
        }
        Ok(VfsChangeBatch {
            reset: true,
            ..Default::default()
        })
    }
}

fn checked_changes(v: &serde_json::Value) -> VfsResult<VfsChangeBatch> {
    let mut out = VfsChangeBatch {
        new_cursor: v["newStartPageToken"].as_str().map(|s| s.to_string()),
        ..Default::default()
    };
    let changes = v["changes"]
        .as_array()
        .ok_or_else(|| invalid("Drive change array is missing"))?;
    for ch in changes {
        let file = &ch["file"];
        let removed = optional_bool(ch, "removed")? || optional_bool(file, "trashed")?;
        let id = ch["fileId"].as_str().map(|s| s.to_string());
        if id.as_deref().is_none_or(|id| id.is_empty()) {
            return Err(invalid("Drive change has no object ID"));
        }
        if !removed
            && (file["id"].as_str() != id.as_deref()
                || file["mimeType"].as_str().is_none_or(|mime| mime.is_empty())
                || file["parents"].as_array().is_none_or(|parents| {
                    parents.len() > 1
                        || parents
                            .iter()
                            .any(|parent| parent.as_str().is_none_or(str::is_empty))
                })
                || GDriveBackend::sync_metadata_problem(file).is_some())
        {
            return Err(invalid(
                "Drive upsert has incomplete or inconsistent metadata",
            ));
        }
        let parent_id = file["parents"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|p| p.as_str())
            .map(|s| s.to_string());
        let name = file["name"].as_str().map(|s| s.to_string());
        let meta = (!removed)
            .then(|| GDriveBackend::meta_from_json(file, name.as_deref()))
            .flatten();
        if !removed && meta.is_none() {
            return Err(invalid("Drive upsert has no usable name"));
        }
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
    Ok(out)
}

fn token<'a>(value: &'a serde_json::Value, name: &str) -> VfsResult<Option<&'a str>> {
    match value.get(name) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .filter(|s| !s.is_empty())
            .map(Some)
            .ok_or_else(|| invalid("Drive feed token is empty or has the wrong type")),
    }
}

fn optional_bool(value: &serde_json::Value, name: &str) -> VfsResult<bool> {
    match value.get(name) {
        None | Some(serde_json::Value::Null) => Ok(false),
        Some(serde_json::Value::Bool(value)) => Ok(*value),
        Some(_) => Err(invalid("Drive change boolean has the wrong type")),
    }
}

fn invalid(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail)
}

#[cfg(test)]
fn parse_changes_value(value: &serde_json::Value) -> VfsChangeBatch {
    checked_changes(value).expect("test fixture has a valid Drive change array")
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
