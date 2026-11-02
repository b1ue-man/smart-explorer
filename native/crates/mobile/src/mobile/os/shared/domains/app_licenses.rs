//! `app.licenses` (plan C4): the license texts of every Rust crate embedded
//! in the Android library, for the "Über" page. `{}` → `{embedded, sha256,
//! crates: [{name, version, license}], text}`; without an embedded file
//! (development build) `text` is the marked notice, `sha256` is null and
//! `crates` is empty.
use serde_json::Value;

use crate::licenses;
use crate::mobile::ApiError;

/// Answers the methods of the `app` domain this module owns; `None` = not one
/// of them. `domains::dispatch` routes the prefix `app` here.
pub(super) fn handle(method: &str) -> Option<Result<Value, ApiError>> {
    match method {
        "app.licenses" => Some(Ok(app_licenses())),
        _ => None,
    }
}

fn app_licenses() -> Value {
    let mut value = licenses::summary_json();
    value["text"] = Value::from(licenses::text().unwrap_or(licenses::MISSING_NOTICE));
    value
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::Value;

    use crate::licenses;

    #[test]
    fn licenses_task_bridge_app_licenses() {
        let value = match super::handle("app.licenses") {
            Some(Ok(value)) => value,
            Some(Err(error)) => panic!("{}: {}", error.kind, error.message),
            None => panic!("app.licenses is not routed"),
        };
        assert!(super::handle("app.unknownMethod").is_none());
        let keys: BTreeSet<&str> = value
            .as_object()
            .expect("JSON object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            BTreeSet::from(["crates", "embedded", "sha256", "text"])
        );
        let crates = value["crates"].as_array().expect("crates is an array");
        match licenses::text() {
            Some(text) => {
                assert_eq!(value["embedded"], Value::Bool(true));
                assert_eq!(value["text"].as_str(), Some(text));
                assert_eq!(value["sha256"].as_str(), licenses::sha256().as_deref());
                assert!(!crates.is_empty(), "crates lists the embedded crates");
                assert_eq!(crates.len(), licenses::crates().len());
                for entry in crates {
                    for field in ["name", "version", "license"] {
                        let field_text = entry[field].as_str().unwrap_or_default();
                        assert!(!field_text.is_empty(), "{field} is empty in {entry}");
                    }
                }
            }
            None => {
                assert_eq!(value["embedded"], Value::Bool(false));
                assert_eq!(value["text"], Value::from(licenses::MISSING_NOTICE));
                assert_eq!(value["sha256"], Value::Null);
                assert!(crates.is_empty());
            }
        }
    }
}
