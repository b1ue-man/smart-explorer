//! Hook registrations and the managed hardening documents
//! (docs/refs/agenten-sperre-elevation-2026-11-28.md §1.3, §1.8, §1.9, §2.1).
use super::hook_install::{self, claude_handler, claude_hooks, codex_trust_hash};
use super::hook_managed::{self, ManagedRequest};
use serde_json::json;
use std::path::{Path, PathBuf};

#[test]
fn masks_lock_task_claude_hook_blocks_failures_and_passes_without_se() {
    let unix = claude_handler(Path::new("/opt/se dir/it's/se"), false);
    assert_eq!(
        unix["command"],
        json!("[ -x '/opt/se dir/it'\\''s/se' ] || exit 0; exec '/opt/se dir/it'\\''s/se' hook claude")
    );
    assert_eq!(unix["onFailure"], json!("block"));
    assert_eq!(unix["timeout"], json!(hook_install::TIMEOUT_SECONDS));
    assert!(unix.get("shell").is_none());
    let windows = claude_handler(Path::new(r"C:\Program Files\Smart Explorer\se.exe"), true);
    assert_eq!(windows["shell"], json!("powershell"));
    assert_eq!(
        windows["command"],
        json!(
            r"if (Test-Path -LiteralPath 'C:\Program Files\Smart Explorer\se.exe' -PathType Leaf) { & 'C:\Program Files\Smart Explorer\se.exe' hook claude; exit $LASTEXITCODE } else { exit 0 }"
        )
    );
    let hooks = claude_hooks(Path::new("/opt/se"), false);
    assert_eq!(hooks["PreToolUse"][0]["matcher"], json!("*"));
    assert_eq!(
        hooks["PreSkillShellCommand"][0]["hooks"][0],
        claude_handler(Path::new("/opt/se"), false)
    );
}

/// The command really lets a call through when the launcher is gone, and
/// passes the exit status of a present program on.
#[cfg(unix)]
#[test]
fn masks_lock_task_claude_hook_command_runs_in_sh() {
    let missing = claude_handler(Path::new("/nonexistent/se-hook-test/se"), false);
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(missing["command"].as_str().unwrap())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(0));
    let present = claude_handler(Path::new("/bin/false"), false);
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(present["command"].as_str().unwrap())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(1));
}

#[test]
fn masks_lock_task_codex_trust_hash_follows_codex_normalization() {
    // SHA-256 of the key-sorted compact JSON of {event_name, matcher,
    // hooks:[{async,command,timeout,type}]} (CX-src discovery.rs hook_hash).
    assert_eq!(
        codex_trust_hash("'/opt/se/se' hook codex"),
        "sha256:07c08f1769ab6cd6acb34b7ec35099e4575eef80e1925276756732fb9b7b155d"
    );
    assert_eq!(
        codex_trust_hash(r#""C:\Program Files\Smart Explorer\se.exe" hook codex"#),
        "sha256:a14028397249e1ccfc9c038d839faa088f229cdf8735c220b56781810691968f"
    );
    let (command, windows) = hook_install::codex_commands(Path::new("/opt/se/se"));
    assert_eq!(command, "'/opt/se/se' hook codex");
    assert_eq!(windows, "\"/opt/se/se\" hook codex");
    assert_eq!(
        hook_install::codex_state_key(Path::new("/home/u/.codex/config.toml"), 2),
        "/home/u/.codex/config.toml:pre_tool_use:2:0"
    );
}

#[test]
fn masks_lock_task_managed_claude_document_hook_and_deny_rules() {
    let request = ManagedRequest {
        launcher: PathBuf::from("/opt/se"),
        deny_paths: vec![
            PathBuf::from("/work/secret/"),
            PathBuf::from(r"C:\Users\u\Projekt"),
            PathBuf::from("relative/ignored"),
        ],
        claude: true,
        codex: false,
    };
    let document = hook_managed::claude_document(&request, false);
    assert_eq!(document["hooks"], claude_hooks(Path::new("/opt/se"), false));
    assert_eq!(
        document["permissions"]["deny"],
        json!([
            "Edit(//c/Users/u/Projekt)",
            "Edit(//c/Users/u/Projekt/**)",
            "Edit(//work/secret)",
            "Edit(//work/secret/**)",
            "Read(//c/Users/u/Projekt)",
            "Read(//c/Users/u/Projekt/**)",
            "Read(//work/secret)",
            "Read(//work/secret/**)",
        ])
    );
    assert_eq!(hook_managed::rule_path(Path::new("/")), Some("//".into()));
    assert_eq!(
        hook_managed::deny_rules(&[PathBuf::from("/")]),
        vec!["Edit(//)", "Edit(//**)", "Read(//)", "Read(//**)"]
    );
    assert!(hook_managed::claude_dropin().ends_with("managed-settings.d/90-smart-explorer.json"));
}

#[test]
fn masks_lock_task_managed_codex_requirements_merge_and_remove() {
    let before = "# admin policy\nallow_managed_hooks_only = true\n\n[[hooks.PreToolUse]]\nmatcher = \"^Bash$\"\n\n[[hooks.PreToolUse.hooks]]\ntype = \"command\"\ncommand = \"/usr/local/bin/policy\"\n";
    let launcher = Path::new("/opt/se");
    let after = hook_managed::codex_document(Some(before), launcher).unwrap();
    assert!(
        after.starts_with("# admin policy\nallow_managed_hooks_only = true\n"),
        "{after}"
    );
    assert!(
        after.contains("command = \"/usr/local/bin/policy\""),
        "{after}"
    );
    assert!(after.contains("hook codex"), "{after}");
    let parsed: toml_edit::DocumentMut = after.parse().unwrap();
    let groups = parsed["hooks"]["PreToolUse"].as_array_of_tables().unwrap();
    assert_eq!(groups.len(), 2);
    assert!(hook_install::is_se_codex_group(groups.get(1).unwrap()));
    // A second run replaces the se group instead of adding one.
    let again = hook_managed::codex_document(Some(&after), Path::new("/opt/se2")).unwrap();
    let parsed: toml_edit::DocumentMut = again.parse().unwrap();
    assert_eq!(
        parsed["hooks"]["PreToolUse"]
            .as_array_of_tables()
            .unwrap()
            .len(),
        2
    );
    assert!(again.contains("'/opt/se2' hook codex"));
    let removed = hook_managed::codex_without_se(&again).unwrap().unwrap();
    assert!(!removed.contains("hook codex"), "{removed}");
    assert!(removed.contains("/usr/local/bin/policy"), "{removed}");
    let only_se = hook_managed::codex_document(None, launcher).unwrap();
    assert_eq!(hook_managed::codex_without_se(&only_se).unwrap(), None);
    assert!(hook_managed::codex_document(Some("[hooks\n"), launcher).is_err());
    assert!(hook_managed::codex_document(Some("hooks = 1\n"), launcher).is_err());
}

#[test]
fn masks_lock_task_managed_request_survives_the_command_line_and_is_checked() {
    let request = ManagedRequest {
        launcher: PathBuf::from("/opt/se \"x\"/se"),
        deny_paths: vec![PathBuf::from("/a b/ä")],
        claude: true,
        codex: true,
    };
    let encoded = hook_managed::encode_request(&request).unwrap();
    assert!(encoded.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(hook_managed::decode_request(&encoded).unwrap(), request);
    assert!(hook_managed::decode_request("zz").is_err());
    assert!(hook_managed::decode_request("abc").is_err());
    // A relative launcher or path is refused before anything is written.
    let relative = ManagedRequest {
        launcher: PathBuf::from("se"),
        ..request.clone()
    };
    assert!(hook_managed::apply_now(&relative).is_err());
    assert!(hook_managed::execute(&["unknown".into()]).is_err());
    assert!(hook_managed::execute(&["remove".into(), "nobody".into()]).is_err());
}
