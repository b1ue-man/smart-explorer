//! P13 owned-profile fixtures; no global profile paths or host CLI are used.
use super::{hash, read, replace};
use super::super::files;
use serde_json::{json, Value};

const MCP: &str = "smart-explorer/.mcp.json";

fn profile(command: &str) -> Value {
    json!({"mcpServers":{"se":{"command":command,"args":["mcp"]}}})
}

#[test]
fn batches_p13_compare_before_replace_preserves_concurrent_editor_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    let original = b"# user comment\ncommand = 'original'\n";
    let editor = b"# user comment\ncommand = 'edited'\n";
    replace(&path, None, original).unwrap();
    std::fs::write(&path, editor).unwrap();
    let error = replace(&path, Some(original), b"replacement").unwrap_err();
    assert!(error.starts_with("integration_conflict:"));
    assert!(error.contains(path.to_string_lossy().as_ref()));
    assert_eq!(read(&path).unwrap().unwrap(), editor);
    assert!(replace(&path, None, original).is_err(), "existing unowned file must survive");
    assert_eq!(read(&path).unwrap().unwrap(), editor);
    replace(&path, Some(editor), original).unwrap();
    assert_eq!(read(&path).unwrap().unwrap(), original);
}

#[test]
fn batches_p13_claude_fingerprint_owns_only_command_and_args() {
    let original = profile("/fixture/launcher");
    let original_bytes = serde_json::to_vec(&original).unwrap();
    let mut extended = original.clone();
    extended["mcpServers"]["se"]["env"] = json!({"USER_SETTING":"keep"});
    extended["mcpServers"]["se"]["allowedTools"] = json!(["se.read"]);
    extended["mcpServers"]["user-server"] = json!({"command":"user-tool"});
    let extended_bytes = serde_json::to_vec(&extended).unwrap();
    assert_eq!(files::owned_fields_fingerprint(MCP, &original_bytes), files::owned_fields_fingerprint(MCP, &extended_bytes));
    let marker = json!({"schema":1,"launcher_path":"/fixture/launcher"});
    assert!(files::own_fields_match(MCP, &extended_bytes, &marker));
    for field in ["command", "args"] {
        let mut changed = extended.clone();
        changed["mcpServers"]["se"][field] = if field == "command" {json!("user-command")} else {json!(["mcp","--user-option"])};
        let changed = serde_json::to_vec(&changed).unwrap();
        assert!(!files::own_fields_match(MCP, &changed, &marker));
        assert_ne!(files::owned_fields_fingerprint(MCP, &changed), files::owned_fields_fingerprint(MCP, &original_bytes));
    }
}

#[test]
fn batches_p13_uninstall_preflight_preserves_shared_mcp_user_fields() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(MCP);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut shared = profile("/fixture/launcher");
    shared["mcpServers"]["se"]["env"] = json!({"KEEP":"user-value"});
    let bytes = serde_json::to_vec(&shared).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    // Exercise both old whole-file markers and current field-only markers.
    for whole_file in [false, true] {
        let mut marker = json!({"schema":1,"fingerprints":{},"field_fingerprints":{}});
        if whole_file {
            marker["fingerprints"][MCP] = json!(hash(&bytes));
        } else {
            marker["field_fingerprints"][MCP] = json!(files::owned_fields_fingerprint(MCP, &bytes).unwrap());
        }
        std::fs::write(directory.path().join(".se-generated.json"), serde_json::to_vec(&marker).unwrap()).unwrap();
        let error = files::verify_owned(directory.path()).unwrap_err();
        assert!(error.contains("contains user fields or additional MCP servers"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn batches_p13_owned_skill_preflight_rejects_modified_or_removed_content() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("SKILL.md");
    let bytes = b"fixture owned skill";
    let marker = json!({"schema":1,"fingerprints":{"SKILL.md":hash(bytes)},"field_fingerprints":{}});
    std::fs::write(directory.path().join(".se-generated.json"), serde_json::to_vec(&marker).unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    files::verify_owned(directory.path()).unwrap();
    std::fs::write(&path, b"user-edited skill").unwrap();
    assert!(files::verify_owned(directory.path()).unwrap_err().contains("owned content changed"));
    assert_eq!(std::fs::read(&path).unwrap(), b"user-edited skill");
    std::fs::remove_file(&path).unwrap();
    assert!(files::verify_owned(directory.path()).unwrap_err().contains("missing"));
    assert!(!path.exists());
}
