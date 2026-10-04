//! Provider payload and query assertions shared by the existing GUI task fixtures.

use super::gui_task_http::Request;
use serde_json::{json, Value};

pub(super) fn object(name: &str, id: &str, folder: bool) -> Value {
    json!({"id": id, "name": name, "parents": ["root"], "trashed": false,
        "mimeType": if folder { super::api::FOLDER_MIME } else { "application/octet-stream" },
        "size": "3", "md5Checksum": "900150983cd24fb0d6963f7d28e17f72",
        "modifiedTime": "2026-09-15T11:41:38Z"})
}

pub(super) fn assert_query(request: &Request, parent: &str, name: &str) {
    let quote = |value: &str| value.replace('\\', "\\\\").replace('\'', "\\'");
    let query = format!(
        "'{}' in parents and name = '{}' and trashed = false",
        quote(parent),
        quote(name)
    );
    assert!(
        request
            .target
            .contains(&format!("q={}", super::core::cloud_urlenc(&query))),
        "{:?}",
        request
    );
}
