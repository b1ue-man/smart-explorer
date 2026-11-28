//! `se hook`: decisions for the input JSON of Claude Code and Codex
//! (docs/refs/agenten-sperre-elevation-2026-11-28.md §1.2, §2.1, §2.2).
use super::hook_policy::{Decision, HookHost};
use super::{decide_bytes, deny_output, Lockdown};
use serde_json::{json, Value};

fn input(tool: &str, tool_input: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "session_id": "s", "cwd": "/work", "hook_event_name": "PreToolUse",
        "permission_mode": "bypassPermissions", "tool_name": tool,
        "tool_input": tool_input, "tool_use_id": "t1"
    }))
    .unwrap()
}

fn decide(host: HookHost, lockdown: Lockdown, bytes: &[u8], allowed: &[&str]) -> Decision {
    let allowed: Vec<String> = allowed.iter().map(|name| name.to_string()).collect();
    decide_bytes(host, lockdown, bytes, &allowed)
}

fn denied(decision: &Decision) -> &str {
    match decision {
        Decision::Deny(reason) => reason,
        Decision::Allow => panic!("expected a refusal"),
    }
}

#[test]
fn masks_lock_task_hook_off_allows_every_call_and_garbage() {
    for bytes in [
        input("Bash", json!({"command": "cat /secret/x"})),
        input("Read", json!({"file_path": "/secret/x"})),
        b"not json".to_vec(),
    ] {
        for host in [HookHost::Claude, HookHost::Codex] {
            assert_eq!(decide(host, Lockdown::Off, &bytes, &[]), Decision::Allow);
        }
    }
}

#[test]
fn masks_lock_task_hook_on_refuses_native_tools_and_names_the_se_tool() {
    let cases = [
        (
            "Read",
            json!({"file_path": "/p/a"}),
            "mcp__plugin_smart-explorer_se__read",
        ),
        (
            "Write",
            json!({"file_path": "/p/a", "content": "x"}),
            "__write",
        ),
        (
            "Edit",
            json!({"file_path": "/p/a", "old_string": "a", "new_string": "b"}),
            "__edit",
        ),
        ("MultiEdit", json!({"file_path": "/p/a"}), "__edit"),
        (
            "NotebookEdit",
            json!({"notebook_path": "/p/n.ipynb"}),
            "__edit",
        ),
        ("Glob", json!({"pattern": "**/*.rs"}), "__search"),
        ("Grep", json!({"pattern": "x"}), "__grep"),
        ("LS", json!({"path": "/p"}), "__list"),
        (
            "LSP",
            json!({"operation": "hover", "filePath": "a.rs"}),
            "__read",
        ),
        ("Bash", json!({"command": "cd /p && ls"}), "__shell"),
        ("PowerShell", json!({"command": "gci"}), "__shell"),
        ("Monitor", json!({"command": "tail -f /p/log"}), "__shell"),
        ("BashOutput", json!({"bash_id": "1"}), "__job_status"),
        ("TaskOutput", json!({"task_id": "1"}), "__job_status"),
        ("EnterWorktree", json!({}), "Smart Explorer agent lockdown"),
        ("SomeFutureTool", json!({}), "Smart Explorer agent lockdown"),
    ];
    for (tool, tool_input, expected) in cases {
        for lockdown in [Lockdown::On, Lockdown::Unknown] {
            let decision = decide(
                HookHost::Claude,
                lockdown,
                &input(tool, tool_input.clone()),
                &[],
            );
            let reason = denied(&decision);
            assert!(reason.contains(expected), "{tool}: {reason}");
            if lockdown == Lockdown::Unknown {
                assert!(reason.contains("fail-closed"), "{tool}: {reason}");
            }
        }
    }
}

#[test]
fn masks_lock_task_hook_on_keeps_se_tools_and_tools_without_file_access() {
    let own = json!({"session_id":"s","hook_event_name":"PreToolUse",
        "tool_name":"mcp__plugin_smart-explorer_se__read","tool_input":{"target":"a"},
        "mcp_server":{"name":"plugin:smart-explorer:se","source":"plugin"}});
    let bytes = serde_json::to_vec(&own).unwrap();
    assert_eq!(
        decide(HookHost::Claude, Lockdown::On, &bytes, &[]),
        Decision::Allow
    );
    // The same name from a user-configured server is not the plugin.
    let mut spoof = own.clone();
    spoof["mcp_server"]["source"] = json!("user");
    let bytes = serde_json::to_vec(&spoof).unwrap();
    assert!(matches!(
        decide(HookHost::Claude, Lockdown::On, &bytes, &[]),
        Decision::Deny(_)
    ));
    for tool in [
        "TodoWrite",
        "Agent",
        "Task",
        "AskUserQuestion",
        "WebSearch",
        "ExitPlanMode",
        "Skill",
        "REPL",
    ] {
        assert_eq!(
            decide(HookHost::Claude, Lockdown::On, &input(tool, json!({})), &[]),
            Decision::Allow,
            "{tool}"
        );
    }
}

#[test]
fn masks_lock_task_hook_codex_tools_and_allowed_servers() {
    let codex = |tool: &str, allowed: &[&str]| {
        decide(
            HookHost::Codex,
            Lockdown::On,
            &input(tool, json!({"command": "x"})),
            allowed,
        )
    };
    for tool in [
        "Bash",
        "apply_patch",
        "view_image",
        "mcp__filesystem__read_file",
    ] {
        let decision = codex(tool, &[]);
        assert!(
            denied(&decision).contains("mcp__smart-explorer__"),
            "{tool}"
        );
    }
    for tool in [
        "mcp__smart-explorer__grep",
        "update_plan",
        "spawn_agent",
        "request_user_input",
    ] {
        assert_eq!(codex(tool, &[]), Decision::Allow, "{tool}");
    }
    assert_eq!(
        codex("mcp__filesystem__read_file", &["filesystem"]),
        Decision::Allow
    );
    assert!(matches!(
        codex("mcp__filesystem2__read_file", &["filesystem"]),
        Decision::Deny(_)
    ));
    let named = json!({"hook_event_name":"PreToolUse","tool_name":"mcp__memory__store",
        "tool_input":{},"mcp_server":{"name":"memory","source":"user"}});
    let bytes = serde_json::to_vec(&named).unwrap();
    assert_eq!(
        decide(HookHost::Claude, Lockdown::On, &bytes, &["memory"]),
        Decision::Allow
    );
    assert!(matches!(
        decide(HookHost::Claude, Lockdown::On, &bytes, &[]),
        Decision::Deny(_)
    ));
}

#[test]
fn masks_lock_task_hook_web_fetch_only_public_addresses() {
    let fetch = |url: &str| {
        decide(
            HookHost::Claude,
            Lockdown::On,
            &input("WebFetch", json!({"url": url, "prompt": "p"})),
            &[],
        )
    };
    for url in [
        "http://127.0.0.1:8080/x",
        "http://user@127.0.0.1/",
        "https://[::1]/",
        "http://[::ffff:10.0.0.1]/",
        "http://10.1.2.3/",
        "http://172.16.0.9/",
        "http://192.168.1.1/",
        "http://169.254.169.254/latest/meta-data",
        "http://100.64.0.1/",
        "http://0.0.0.0/",
        "http://[fd00::1]/",
        "http://[fe80::1]/",
        "http://localhost/",
        "http://printer.local/",
        "http://intranet/",
        "file:///etc/passwd",
        "ftp://example.org/",
        "not a url",
    ] {
        assert!(matches!(fetch(url), Decision::Deny(_)), "{url}");
    }
    // Literal public addresses need no name resolution.
    for url in [
        "https://93.184.215.14/",
        "http://[2606:2800:21f:cb07:6820:80da:af6b:8b2c]:8443/a",
    ] {
        assert_eq!(fetch(url), Decision::Allow, "{url}");
    }
    assert_eq!(
        super::hook_web::host_and_port("https://u:p@Example.ORG.:8443/a?b#c").unwrap(),
        ("example.org".to_string(), 8443)
    );
    assert!(matches!(
        decide(
            HookHost::Claude,
            Lockdown::On,
            &input("WebFetch", json!({})),
            &[]
        ),
        Decision::Deny(_)
    ));
}

#[test]
fn masks_lock_task_hook_events_resources_and_output_shape() {
    let skill = json!({"hook_event_name":"PreSkillShellCommand","command":"cat /p/a"});
    let bytes = serde_json::to_vec(&skill).unwrap();
    assert!(denied(&decide(HookHost::Claude, Lockdown::On, &bytes, &[])).contains("skills"));
    let other = serde_json::to_vec(&json!({"hook_event_name":"SessionStart"})).unwrap();
    assert_eq!(
        decide(HookHost::Claude, Lockdown::On, &other, &[]),
        Decision::Allow
    );
    assert!(matches!(
        decide(HookHost::Codex, Lockdown::On, b"{", &[]),
        Decision::Deny(_)
    ));
    let no_tool = serde_json::to_vec(&json!({"hook_event_name":"PreToolUse"})).unwrap();
    assert!(matches!(
        decide(HookHost::Codex, Lockdown::On, &no_tool, &[]),
        Decision::Deny(_)
    ));
    let resources = |server: Value, allowed: &[&str]| {
        decide(
            HookHost::Claude,
            Lockdown::On,
            &input("ReadMcpResourceTool", json!({"server": server, "uri": "x"})),
            allowed,
        )
    };
    assert_eq!(
        resources(json!("plugin:smart-explorer:se"), &[]),
        Decision::Allow
    );
    assert_eq!(resources(json!("docs"), &["docs"]), Decision::Allow);
    assert!(matches!(resources(json!("docs"), &[]), Decision::Deny(_)));
    assert!(matches!(resources(Value::Null, &[]), Decision::Deny(_)));
    assert_eq!(
        deny_output("why"),
        json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"why"}})
    );
}
