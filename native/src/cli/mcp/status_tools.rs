//! `status` and `connections`: the same documents as `se doctor --json`,
//! `se share status --json` and `se connections list --json`, condensed and
//! annotated with the target to use and how commands run there.
use super::response;
use super::session::Session;
use rmcp::model::CallToolResult;
use serde_json::{json, Value};

pub(super) fn status(session: &Session) -> Result<CallToolResult, String> {
    let (installation, healthy) = crate::cli::doctor::doctor_value()?;
    let share = match crate::cli::share::status_value() {
        Ok(value) => share_summary(&value),
        Err(error) => json!({ "error": error }),
    };
    let running = session
        .jobs
        .list()
        .iter()
        .filter(|job| job.end().is_none())
        .count();
    Ok(response::ok(json!({
        "healthy": healthy,
        "installation": installation,
        "share": share,
        "mcp": {
            "server": "smart-explorer",
            "version": env!("CARGO_PKG_VERSION"),
            "executable": session.executable,
            "session_directory": session.directory,
            "running_jobs": running,
        },
        "plugins": crate::cli::plugin::status_value(),
    })))
}

/// The parts of `se share status --json` an agent needs; recent events and
/// historic requests stay in the full command (`se` tool).
fn share_summary(status: &Value) -> Value {
    let count = |key: &str| status[key].as_array().map_or(0, Vec::len);
    json!({
        "worker": status["worker"],
        "contacts": status["contacts"],
        "rooms": status["rooms"],
        "pending_requests": status["pending_requests"],
        "requests": count("requests"),
        "grants": count("grants"),
        "full_status": "se tool with [\"share\",\"status\",\"--json\"]",
    })
}

pub(super) fn connections() -> Result<CallToolResult, String> {
    let rows = crate::cli::connections::connections_value()?;
    let mut items = match rows {
        Value::Array(items) => items,
        _ => Vec::new(),
    };
    for row in &mut items {
        annotate(row);
    }
    Ok(response::ok(json!({ "connections": items })))
}

fn annotate(row: &mut Value) {
    let protocol = row["protocol"].as_str().unwrap_or_default().to_string();
    let kind = row["kind"].as_str().unwrap_or_default().to_string();
    let (target, exec) = if protocol == "share" {
        let endpoint = row["endpoint"]
            .as_str()
            .or_else(|| row["account"].as_str())
            .unwrap_or_default()
            .trim_end_matches('/')
            .to_string();
        let exec = matches!(kind.as_str(), "direct" | "room-member").then_some("share");
        (format!("{endpoint}/"), exec)
    } else {
        let label = row["label"].as_str().unwrap_or_default();
        (format!("@{label}:/"), (protocol == "sftp").then_some("ssh"))
    };
    if let Value::Object(fields) = row {
        fields.insert("target".into(), json!(target));
        fields.insert("exec".into(), json!(exec));
    }
}

#[cfg(test)]
mod tests {
    use super::{annotate, share_summary};
    use serde_json::json;

    #[test]
    fn mcp_task_connections_name_their_target_and_exec_kind() {
        let mut sftp = json!({"protocol": "sftp", "label": "prod", "account": "sftp://a@h:22/srv"});
        annotate(&mut sftp);
        assert_eq!(sftp["target"], "@prod:/");
        assert_eq!(sftp["exec"], "ssh");
        let mut dav = json!({"protocol": "webdav", "label": "cloud"});
        annotate(&mut dav);
        assert_eq!(dav["exec"], json!(null));
        let mut peer =
            json!({"protocol": "share", "kind": "direct", "endpoint": "share://direct/c1"});
        annotate(&mut peer);
        assert_eq!(peer["target"], "share://direct/c1/");
        assert_eq!(peer["exec"], "share");
        let summary =
            share_summary(&json!({"worker": {"running": true}, "grants": [1, 2], "events": [1]}));
        assert_eq!(summary["grants"], 2);
        assert!(summary.get("events").is_none());
    }
}
