//! Tool results. `structuredContent` always carries the complete result:
//! Codex shows the model only that JSON when it is present
//! (docs/refs/codex-mcp-spec-rmcp-2026-10-12.md §1); the text block repeats it
//! for clients that read content only, as the MCP specification recommends.
use rmcp::model::CallToolResult;
use serde_json::{json, Value};

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

/// The page budget a call asked for with `max_bytes`, or [`PAGE_BYTES`].
pub(super) fn page_budget(requested: Option<u64>) -> usize {
    requested
        .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
        .unwrap_or(PAGE_BYTES)
        .max(MIN_PAGE_BYTES)
}

#[cfg(test)]
mod tests {
    use super::{error_with, ok, page_budget, PAGE_BYTES};
    use serde_json::json;

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
