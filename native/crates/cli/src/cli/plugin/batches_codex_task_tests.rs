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
    set_owned(
        &mut doc["mcp_servers"][SERVER],
        "command",
        value("/fixture/new-launcher"),
    );
    let rendered = doc.to_string();
    assert!(rendered.starts_with("# user heading\nmodel = 'user-model'\n"));
    for preserved in [
        "# command comment",
        "args = ['mcp']",
        "enabled_tools = ['se.read']",
        "startup_timeout_sec = 41",
        "USER_SETTING = 'keep'",
        "[mcp_servers.user_server]\ncommand = 'user-tool'\nargs = ['--keep']",
    ] {
        assert!(rendered.contains(preserved), "lost {preserved}");
    }
    let parsed = document(Some(rendered.as_bytes())).unwrap();
    assert_eq!(
        fields(&parsed["mcp_servers"][SERVER]).unwrap(),
        json!({"command":"/fixture/new-launcher","args":["mcp"]})
    );
    assert!(!owner_matches(&parsed["mcp_servers"][SERVER], &owner));
}

#[test]
fn batches_p13_codex_ownership_rejects_command_args_changes_but_keeps_user_options() {
    let original = document(Some(CONFIG.as_bytes())).unwrap();
    let expected = fields(&original["mcp_servers"][SERVER]).unwrap();
    let owner =
        json!({"entry_sha256":super::managed::hash(&serde_json::to_vec(&expected).unwrap())});
    let mut changed = document(Some(CONFIG.as_bytes())).unwrap();
    changed["mcp_servers"][SERVER]["startup_timeout_sec"] = value(73);
    changed["mcp_servers"][SERVER]["env"]["USER_SETTING"] = value("edited");
    assert!(owner_matches(&changed["mcp_servers"][SERVER], &owner));
    changed["mcp_servers"][SERVER]["command"] = value("user-command");
    assert!(!owner_matches(&changed["mcp_servers"][SERVER], &owner));
    let mut changed = document(Some(CONFIG.as_bytes())).unwrap();
    changed["mcp_servers"][SERVER]["args"] =
        value(toml_edit::Array::from_iter(["mcp", "--changed"]));
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
    assert!(
        !path.exists(),
        "failed first registration must restore absence"
    );

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

#[test]
#[cfg(not(windows))]
fn batches_p13_legacy_codex_environment_is_owned_before_and_after_migration() {
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("se");
    let marker = json!({"version":"0.5.184","executable":executable});
    let config=format!("# user's heading\n[mcp_servers.smart-explorer]\ncommand = {}\nargs = ['mcp']\n[mcp_servers.smart-explorer.env]\nXDG_DATA_HOME = '/fixture/data'\nXDG_RUNTIME_DIR = '/fixture/runtime'\nDBUS_SESSION_BUS_ADDRESS = 'unix:path=/fixture/bus'\n[mcp_servers.user_server]\ncommand = 'keep'\n",serde_json::to_string(&executable.to_string_lossy()).unwrap());
    let mut doc = document(Some(config.as_bytes())).unwrap();
    let table = &doc["mcp_servers"][SERVER];
    assert!(super::legacy_entry_matches(
        table,
        &marker,
        super::files::LEGACY_SKILL.as_bytes()
    ));
    super::verify_removable_entry(table, None, true).unwrap();
    let managed_env = super::legacy_managed_env(table);
    assert_eq!(
        managed_env,
        json!({"XDG_DATA_HOME":"/fixture/data","XDG_RUNTIME_DIR":"/fixture/runtime","DBUS_SESSION_BUS_ADDRESS":"unix:path=/fixture/bus"})
    );
    set_owned(
        &mut doc["mcp_servers"][SERVER],
        "command",
        value("/fixture/launcher"),
    );
    let table = &doc["mcp_servers"][SERVER];
    let owner = json!({"managed_env":managed_env,"entry_sha256":super::managed::hash(&serde_json::to_vec(&fields(table).unwrap()).unwrap())});
    super::verify_removable_entry(table, Some(&owner), false).unwrap();
    super::comments::remove_server(&mut doc).unwrap();
    let result = doc.to_string();
    assert!(result.starts_with("# user's heading\n"));
    assert!(result.contains("[mcp_servers.user_server]\ncommand = 'keep'"));
}

#[cfg(windows)]
#[test]
fn batches_p13_legacy_windows_codex_does_not_adopt_user_unix_environment() {
    let doc=document(Some(b"[mcp_servers.smart-explorer]\ncommand = 'fixture-se'\nargs = ['mcp']\n[mcp_servers.smart-explorer.env]\nXDG_DATA_HOME = 'user-data'\n")).unwrap();
    let table = &doc["mcp_servers"][SERVER];
    assert_eq!(super::legacy_managed_env(table), json!({}));
    assert!(super::verify_removable_entry(table, None, true)
        .unwrap_err()
        .contains("user environment"));
}

#[test]
fn batches_p13_codex_uninstall_retains_user_env_filters_and_changed_owned_values() {
    let config=b"[mcp_servers.smart-explorer]\ncommand = '/fixture/se'\nargs = ['mcp']\n[mcp_servers.smart-explorer.env]\nXDG_DATA_HOME = '/fixture/data'\n";
    let original = document(Some(config)).unwrap();
    let table = &original["mcp_servers"][SERVER];
    let owner = json!({"managed_env":super::legacy_managed_env(table),"entry_sha256":super::managed::hash(&serde_json::to_vec(&fields(table).unwrap()).unwrap())});
    for change in ["env", "filter", "managed_env", "command", "args"] {
        let mut doc = document(Some(config)).unwrap();
        match change {
            "env" => doc["mcp_servers"][SERVER]["env"]["USER_SETTING"] = value("keep"),
            "filter" => {
                doc["mcp_servers"][SERVER]["enabled_tools"] =
                    value(toml_edit::Array::from_iter(["se.read"]))
            }
            "managed_env" => {
                doc["mcp_servers"][SERVER]["env"]["XDG_DATA_HOME"] = value("/user/changed")
            }
            "command" => doc["mcp_servers"][SERVER]["command"] = value("user-command"),
            _ => {
                doc["mcp_servers"][SERVER]["args"] =
                    value(toml_edit::Array::from_iter(["mcp", "--user"]))
            }
        }
        let before = doc.to_string();
        assert!(
            super::verify_removable_entry(&doc["mcp_servers"][SERVER], Some(&owner), false)
                .is_err()
        );
        assert_eq!(doc.to_string(), before);
        if matches!(change, "env" | "filter") {
            assert!(
                super::verify_removable_entry(&doc["mcp_servers"][SERVER], None, true).is_err()
            );
        }
    }
    assert!(super::verify_removable_entry(table, None, false).is_err());
    let mut missing = owner.clone();
    missing.as_object_mut().unwrap().remove("managed_env");
    assert!(super::verify_removable_entry(table, Some(&missing), false).is_err());
}

#[test]
fn batches_p13_missing_codex_host_keeps_legacy_registration_evidence() {
    let root = tempfile::tempdir().unwrap();
    let skill = root.path().join("SKILL.md");
    let marker = root.path().join(".se-generated.json");
    let legacy_marker =
        serde_json::to_vec(&json!({"version":"0.5.184","executable":root.path().join("se")}))
            .unwrap();
    std::fs::write(&skill, super::files::LEGACY_SKILL).unwrap();
    std::fs::write(&marker, &legacy_marker).unwrap();
    super::preflight_skill_at(root.path()).unwrap();
    assert_eq!(std::fs::read(&marker).unwrap(), legacy_marker);
    assert_eq!(
        std::fs::read(&skill).unwrap(),
        super::files::LEGACY_SKILL.as_bytes()
    );
    std::fs::write(&skill, b"user-edited skill").unwrap();
    assert!(super::preflight_skill_at(root.path()).is_err());
    assert_eq!(std::fs::read(skill).unwrap(), b"user-edited skill");
    assert_eq!(std::fs::read(marker).unwrap(), legacy_marker);
}

#[test]
fn batches_p13_codex_removal_preserves_owned_table_comments_without_copying_string_hashes() {
    let config = "# user's heading\n[mcp_servers.smart-explorer] # server note\n# command note\ncommand = '/fixture/#not-a-comment' # inline command note\nargs = [\n  'mcp', # argument note\n  # trailing argument note\n]\n# environment note\n[mcp_servers.smart-explorer.env] # env header note\nXDG_DATA_HOME = '/fixture/#private' # value note\n\n# user's server\n[mcp_servers.user_server]\ncommand = 'keep#literal' # keep inline\n";
    let mut doc = document(Some(config.as_bytes())).unwrap();
    super::comments::remove_server(&mut doc).unwrap();
    let rendered = doc.to_string();
    assert!(rendered.starts_with("# user's heading\n"));
    for comment in [
        "# server note", "# command note", "# inline command note",
        "# argument note", "# trailing argument note", "# environment note",
        "# env header note", "# value note", "# user's server", "# keep inline",
    ] {
        assert_eq!(rendered.matches(comment).count(), 1, "{comment}: {rendered}");
    }
    assert!(!rendered.contains("#not-a-comment"));
    assert!(!rendered.contains("#private"));
    assert!(rendered.contains("[mcp_servers.user_server]\ncommand = 'keep#literal' # keep inline"));
    let reparsed = document(Some(rendered.as_bytes())).unwrap();
    assert!(reparsed["mcp_servers"].get(SERVER).is_none());
    assert_eq!(reparsed["mcp_servers"]["user_server"]["command"].as_str(), Some("keep#literal"));
    super::comments::remove_server(&mut doc).unwrap();
    assert_eq!(doc.to_string(), rendered, "retry must not duplicate comments");
}

#[test]
fn batches_p13_codex_inline_and_dotted_removal_keeps_user_document_parseable() {
    for config in [
        "# heading\n[mcp_servers]\n# entry note\nsmart-explorer = {command = 'se', args = ['mcp']} # inline note\nuser_server = {command = 'keep'}\n",
        "# heading\nmcp_servers.smart-explorer.command = 'se' # command note\nmcp_servers.smart-explorer.args = ['mcp']\nmcp_servers.user_server.command = 'keep'\n",
        "# heading\n[mcp_servers.smart-explorer]\ncommand = 'se'\nargs = ['mcp'] # inline note\n# final document note\n",
    ] {
        let mut doc = document(Some(config.as_bytes())).unwrap();
        super::comments::remove_server(&mut doc).unwrap();
        let rendered = doc.to_string();
        assert!(rendered.starts_with("# heading\n"), "{rendered}");
        let reparsed = document(Some(rendered.as_bytes())).unwrap();
        assert!(reparsed.get("mcp_servers").and_then(|t| t.get(SERVER)).is_none());
        if config.contains("'keep'") {
            assert_eq!(reparsed["mcp_servers"]["user_server"]["command"].as_str(), Some("keep"));
        }
        if config.contains("# final document note") {
            assert!(rendered.contains("# final document note"));
        }
    }
}
