//! The Codex user hook in `config.toml`: added beside the user's hooks with
//! its trust entry, replaced in place, removed without touching the rest.
use super::{apply, remove, status};
use crate::cli::hook::hook_install;
use std::path::Path;
use toml_edit::DocumentMut;

const USER: &str = "# my config\nmodel = \"m\"\n\n[[hooks.PreToolUse]]\nmatcher = \"^Bash$\"\n\n[[hooks.PreToolUse.hooks]]\ntype = \"command\"\ncommand = \"my-policy\" # mine\n\n[hooks.state.\"x:pre_tool_use:0:0\"]\ntrusted_hash = \"sha256:user\"\n";

#[test]
fn masks_lock_task_codex_user_hook_is_added_trusted_and_removed_cleanly() {
    let config = Path::new("/home/u/.codex/config.toml");
    let launcher = Path::new("/opt/se/se");
    let mut doc: DocumentMut = USER.parse().unwrap();
    let record = apply(&mut doc, config, launcher, None).unwrap();
    let text = doc.to_string();
    assert!(text.starts_with("# my config\nmodel = \"m\"\n"), "{text}");
    assert!(text.contains("command = \"my-policy\" # mine"), "{text}");
    let key = "/home/u/.codex/config.toml:pre_tool_use:1:0";
    assert_eq!(record["state_key"], key);
    let (command, command_windows) = hook_install::codex_commands(launcher);
    let effective = if cfg!(windows) {
        command_windows
    } else {
        command
    };
    let trusted = hook_install::codex_trust_hash(&effective);
    assert_eq!(record["trusted_hash"], trusted.as_str());
    assert_eq!(
        doc["hooks"]["state"][key]["trusted_hash"].as_str(),
        Some(trusted.as_str())
    );
    assert_eq!(
        doc["hooks"]["state"]["x:pre_tool_use:0:0"]["trusted_hash"].as_str(),
        Some("sha256:user")
    );
    let state = status(&doc, config, launcher);
    assert_eq!(state["present"], true);
    assert_eq!(state["current"], true);
    assert_eq!(state["trusted"], true);
    // Idempotent; a new launcher replaces the group in place.
    let again = apply(
        &mut doc,
        config,
        launcher,
        Some(&serde_json::json!({"hook": record})),
    )
    .unwrap();
    assert_eq!(doc.to_string(), text);
    let moved = Path::new("/opt/se2/se");
    let owner = serde_json::json!({"hook": again});
    apply(&mut doc, config, moved, Some(&owner)).unwrap();
    let groups = doc["hooks"]["PreToolUse"].as_array_of_tables().unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(status(&doc, config, launcher)["current"], false);
    let owner = serde_json::json!({"hook": apply(&mut doc, config, moved, Some(&owner)).unwrap()});
    assert!(remove(&mut doc, Some(&owner)));
    let removed = doc.to_string();
    assert!(!removed.contains("hook codex"), "{removed}");
    assert!(removed.contains("my-policy"), "{removed}");
    assert!(removed.contains("sha256:user"), "{removed}");
    assert!(!removed.contains(key), "{removed}");
    assert!(!remove(&mut doc, Some(&owner)));
    assert_eq!(status(&doc, config, launcher)["present"], false);
}

#[test]
fn masks_lock_task_codex_user_hook_refuses_foreign_shapes_without_changes() {
    let config = Path::new("/c/config.toml");
    for text in [
        "hooks = 3\n",
        "[hooks]\nPreToolUse = 1\n",
        "[hooks]\nstate = 2\n",
    ] {
        let mut doc: DocumentMut = text.parse().unwrap();
        assert!(
            apply(&mut doc, config, Path::new("/opt/se"), None).is_err(),
            "{text}"
        );
        assert_eq!(doc.to_string(), text);
    }
    let mut empty = DocumentMut::new();
    apply(&mut empty, config, Path::new("/opt/se"), None).unwrap();
    let text = empty.to_string();
    assert!(text.contains("[[hooks.PreToolUse]]"), "{text}");
    assert!(
        text.contains("[hooks.state.\"/c/config.toml:pre_tool_use:0:0\"]"),
        "{text}"
    );
    assert!(!text.contains("[hooks]\n"), "{text}");
}
