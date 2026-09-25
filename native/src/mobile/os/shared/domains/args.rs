//! Argument access and error helpers shared by the domain methods.
use std::fmt::Display;
use std::io;

use serde_json::Value;

use crate::mobile::{ApiError, Runtime};

pub(super) fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new("invalid", message)
}

pub(super) fn canceled(message: impl Into<String>) -> ApiError {
    ApiError::new("canceled", message)
}

/// `io::Error` with a German context prefix; the kind follows the error.
pub(super) fn io_error(context: &str, error: io::Error) -> ApiError {
    let message = format!("{context}: {error}");
    let mut api = ApiError::from(error);
    api.message = message;
    api
}

/// A plain-text failure from a core helper, with a German context prefix.
pub(super) fn text_error(kind: &'static str, context: &str, error: impl Display) -> ApiError {
    ApiError::new(kind, format!("{context}: {error}"))
}

pub(super) fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, ApiError> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("Parameter „{key}“ fehlt.")))
}

/// Missing, `null` and non-string values read as `None`.
pub(super) fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

pub(super) fn bool_arg(args: &Value, key: &str) -> Result<bool, ApiError> {
    args.get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid(format!("Parameter „{key}“ fehlt.")))
}

pub(super) fn opt_bool(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(default)
}

pub(super) fn i64_arg(args: &Value, key: &str) -> Result<i64, ApiError> {
    args.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid(format!("Parameter „{key}“ fehlt oder ist keine Zahl.")))
}

pub(super) fn opt_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(Value::as_i64)
}

pub(super) fn string_list(args: &Value, key: &str) -> Result<Vec<String>, ApiError> {
    let items = args
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(format!("Parameter „{key}“ fehlt.")))?;
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| invalid(format!("Parameter „{key}“ enthält keinen Text.")))
        })
        .collect()
}

/// `zip://` and `trash://` never reach desktop formats (jobs, Share exports).
pub(super) fn reject_app_internal(location: &str) -> Result<(), ApiError> {
    if Runtime::is_app_internal(location) {
        return Err(invalid(
            "Orte in ZIP-Archiven oder im Papierkorb sind hier nicht möglich.",
        ));
    }
    Ok(())
}

pub(super) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

pub(super) fn now_secs() -> i64 {
    now_ms() / 1000
}
