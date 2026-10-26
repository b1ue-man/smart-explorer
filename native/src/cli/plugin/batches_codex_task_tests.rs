//! P13 Codex field boundary only: no HOME/CODEX_HOME changes and no CLI process.
use super::{document, fields, owner_matches, set_owned, SERVER};
use serde_json::json;
use toml_edit::value;

const CONFIG: &str = "# user heading\nmodel = 'user-model'\n\n[mcp_servers.smart-explorer]\ncommand = '/fixture/old-launcher' # command comment\nargs = ['mcp']\nenabled_tools = ['se.read']\nstartup_timeout_sec = 41\n\n[mcp_servers.smart-explorer.env]\nUSER_SETTING = 'keep'\n\n[mcp_servers.user_server]\ncommand = 'user-tool'\nargs = ['--keep']\n";

#[test]
fn batches_p13_codex_owned_command_edit_preserves_comments_filters_env_and_servers() {
    let mut doc = document(Some(CONFIG.as_bytes())).unwrap();
    let original_fields = fields(&doc["mcp_servers"][SERVER]).unwrap();
    let owner = json!({"entry_sha256":super::managed::hash(&serde_json::to_vec(&original_fields).unwrap())});
    assert!(owner_matches(&doc["mcp_servers"][SERVER], &owner));
    set_owned(&mut doc["mcp_servers"][SERVER], "command", value("/fixture/new-launcher"));
    let rendered = doc.to_string();
    assert!(rendered.starts_with("# user heading\nmodel = 'user-model'\n"));
    for preserved in [
        "# command comment", "args = ['mcp']", "enabled_tools = ['se.read']",
        "startup_timeout_sec = 41", "USER_SETTING = 'keep'",
        "[mcp_servers.user_server]\ncommand = 'user-tool'\nargs = ['--keep']",
    ] {
        assert!(rendered.contains(preserved), "lost {preserved}");
    }
    let parsed = document(Some(rendered.as_bytes())).unwrap();
    assert_eq!(fields(&parsed["mcp_servers"][SERVER]).unwrap(), json!({"command":"/fixture/new-launcher","args":["mcp"]}));
    assert!(!owner_matches(&parsed["mcp_servers"][SERVER], &owner));
}

#[test]
fn batches_p13_codex_ownership_rejects_command_args_changes_but_keeps_user_options() {
    let original = document(Some(CONFIG.as_bytes())).unwrap();
    let expected = fields(&original["mcp_servers"][SERVER]).unwrap();
    let owner = json!({"entry_sha256":super::managed::hash(&serde_json::to_vec(&expected).unwrap())});
    let mut changed = document(Some(CONFIG.as_bytes())).unwrap();
    changed["mcp_servers"][SERVER]["startup_timeout_sec"] = value(73);
    changed["mcp_servers"][SERVER]["env"]["USER_SETTING"] = value("edited");
    assert!(owner_matches(&changed["mcp_servers"][SERVER], &owner));
    changed["mcp_servers"][SERVER]["command"] = value("user-command");
    assert!(!owner_matches(&changed["mcp_servers"][SERVER], &owner));
    let mut changed = document(Some(CONFIG.as_bytes())).unwrap();
    changed["mcp_servers"][SERVER]["args"] = value(toml_edit::Array::from_iter(["mcp", "--changed"]));
    assert!(!owner_matches(&changed["mcp_servers"][SERVER], &owner));
    changed["mcp_servers"][SERVER]["args"] = value(42);
    assert!(fields(&changed["mcp_servers"][SERVER]).is_err());
    assert!(document(Some(b"[invalid")).is_err());
}

#[test]
fn batches_p13_codex_failed_registration_restores_absence_or_previous_owned_bytes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config.toml");
    let staged = b"[mcp_servers.smart-explorer]\ncommand = 'new'\nargs = ['mcp']\n";
    std::fs::write(&path, staged).unwrap();
    super::restore_owned(&path, None, staged).unwrap();
    assert!(!path.exists(), "failed first registration must restore absence");

    std::fs::write(&path, staged).unwrap();
    super::restore_owned(&path, Some(CONFIG.as_bytes()), staged).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), CONFIG.as_bytes());

    let edited = b"# user changed this while registration failed\nmodel = 'keep'\n";
    for before in [None, Some(CONFIG.as_bytes())] {
        std::fs::write(&path, edited).unwrap();
        super::restore_owned(&path, before, staged).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), edited);
    }
}
