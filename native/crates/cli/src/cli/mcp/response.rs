//! Tool results. `structuredContent` always carries the complete result:
//! Codex shows the model only that JSON when it is present
//! (docs/refs/codex-mcp-spec-rmcp-2026-10-12.md §1); the text block repeats it
//! for clients that read content only, as the MCP specification recommends.
//!
//! With an active mask list every string of a result is masked exactly once
//! (plan 2026-11-28, contract 4): a tool that masks its answer itself returns
//! [`masked`] or [`masked_error_with`], whose private `_meta` mark tells the
//! mask layer (`mask_layer::finish`) to pass it on unchanged; every other
//! result is masked there.
use rmcp::model::{CallToolResult, MetaObject};
use serde_json::{json, Value};

/// `_meta` key of a result whose strings the tool masked itself. The mask
/// layer removes it before the result leaves the server.
pub(super) const MASKED_META: &str = "smart-explorer/masked";

/// Default payload budget of one answer. Codex cuts every tool result at
/// 10 000 tokens and Claude Code warns above 10 000 tokens; 20 000 bytes stay
/// below both even for dense JSON at about two characters per token.
pub(super) const PAGE_BYTES: usize = 20_000;
/// Smallest budget a caller may request; smaller pages only add round trips.
const MIN_PAGE_BYTES: usize = 1_024;

pub(super) fn ok(value: Value) -> CallToolResult {
    CallToolResult::structured(value)
}

pub(super) fn error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::structured_error(json!({ "error": message.into() }))
}

/// Error with additional machine-readable fields (for example the current
/// `etag` after a failed precondition).
pub(super) fn error_with(message: impl Into<String>, mut details: Value) -> CallToolResult {
    if let Some(object) = details.as_object_mut() {
        object.insert("error".to_string(), Value::String(message.into()));
        CallToolResult::structured_error(details)
    } else {
        error(message)
    }
}

/// A result whose strings the tool already masked with the active mask
/// lists (placeholders in names, paths, content and messages).
pub(super) fn masked(value: Value) -> CallToolResult {
    mark_masked(ok(value))
}

/// An error whose `message` and `details` the tool already masked.
pub(super) fn masked_error_with(message: impl Into<String>, details: Value) -> CallToolResult {
    mark_masked(error_with(message, details))
}

/// Marks `result` as masked by the tool itself.
pub(super) fn mark_masked(result: CallToolResult) -> CallToolResult {
    let mut meta = result.meta.clone().unwrap_or_else(MetaObject::new);
    meta.insert(MASKED_META.into(), json!(true));
    result.with_meta(Some(meta))
}

/// Removes the mark of [`masked`]; returns whether it was there.
pub(super) fn take_masked_mark(result: &mut CallToolResult) -> bool {
    let Some(meta) = result.meta.as_mut() else {
        return false;
    };
    let marked = meta.remove(MASKED_META).is_some();
    if meta.is_empty() {
        result.meta = None;
    }
    marked
}

/// The page budget a call asked for with `max_bytes`, or [`PAGE_BYTES`].
pub(super) fn page_budget(requested: Option<u64>) -> usize {
    requested
        .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
        .unwrap_or(PAGE_BYTES)
        .max(MIN_PAGE_BYTES)
}

#[cfg(test)]
mod tests {
    use super::{
        error_with, masked, masked_error_with, ok, page_budget, take_masked_mark, PAGE_BYTES,
    };
    use serde_json::json;

    #[test]
    fn mcp_masks_task_masked_results_carry_a_private_mark() {
        let mut plain = ok(json!({"path": "/a"}));
        assert!(!take_masked_mark(&mut plain));
        assert!(plain.meta.is_none());
        let mut done = masked(json!({"path": "/a"}));
        assert!(take_masked_mark(&mut done));
        // The mark never reaches the client; the answer is unchanged.
        assert!(done.meta.is_none());
        assert_eq!(done.structured_content, Some(json!({"path": "/a"})));
        assert_eq!(done.is_error, Some(false));
        let mut failed = masked_error_with("changed", json!({"etag": "e"}));
        assert!(take_masked_mark(&mut failed));
        assert_eq!(failed.is_error, Some(true));
        assert_eq!(
            failed.structured_content,
            Some(json!({"etag": "e", "error": "changed"}))
        );
    }

    #[test]
    fn mcp_task_results_carry_structured_and_text_json() {
        let result = ok(json!({"path": "/a"}));
        assert_eq!(result.structured_content, Some(json!({"path": "/a"})));
        assert_eq!(result.is_error, Some(false));
        let failed = error_with("changed", json!({"etag": "e2"}));
        assert_eq!(failed.is_error, Some(true));
        assert_eq!(
            failed.structured_content,
            Some(json!({"etag": "e2", "error": "changed"}))
        );
        assert_eq!(page_budget(None), PAGE_BYTES);
        assert_eq!(page_budget(Some(10)), 1_024);
        assert_eq!(page_budget(Some(64_000)), 64_000);
    }
}
