//! The catalog of `se mcp`: exact tool list, read-only marks, the tools
//! Claude Code loads upfront and the instruction limits of the hosts.
use super::{tools, INSTRUCTIONS};

#[test]
fn mcp_task_catalog_is_complete_and_within_host_limits() {
    let tools = tools();
    let names = tools
        .iter()
        .map(|tool| tool.name.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "status",
            "connections",
            "root",
            "list",
            "stat",
            "read",
            "write",
            "edit",
            "mkdir",
            "remove",
            "copy",
            "move",
            "search",
            "grep",
            "exec",
            "job_status",
            "job_input",
            "job_cancel",
            "se",
            "sessions",
            "screen",
            // Added by 1aea1216 and cf6f47e9 (docs/oberflaechen/fernzugriff.md).
            "screen_window",
            "screen_input",
            "tcp_forward",
            "apps",
            "rdp",
            "login"
        ]
    );
    assert!(INSTRUCTIONS.len() <= 2_048, "{}", INSTRUCTIONS.len());
    for tool in &tools {
        let description = tool.description.as_deref().unwrap_or_default();
        assert!(
            description.len() <= 2_048,
            "{} description too long",
            tool.name
        );
        let annotations = tool.annotations.as_ref().expect("annotations");
        let read_only = annotations.read_only_hint == Some(true);
        assert_eq!(
            read_only,
            matches!(
                tool.name.as_ref(),
                "status"
                    | "connections"
                    | "root"
                    | "list"
                    | "stat"
                    | "read"
                    | "search"
                    | "grep"
                    | "job_status"
            ),
            "{}",
            tool.name
        );
        assert_eq!(tool.input_schema["type"], "object");
        let upfront = tool
            .meta
            .as_ref()
            .and_then(|meta| meta.get("anthropic/alwaysLoad"))
            .cloned();
        // The file tools and the remote-command tools load upfront.
        let deferred = matches!(
            tool.name.as_ref(),
            "status"
                | "se"
                | "screen"
                | "screen_window"
                | "screen_input"
                | "tcp_forward"
                | "apps"
                | "rdp"
                | "login"
        );
        assert_eq!(
            upfront,
            (!deferred).then_some(serde_json::Value::Bool(true)),
            "{}",
            tool.name
        );
    }
    // Codex keeps the first 512 characters; the priority over the host's
    // own file tools must be complete there.
    let first = INSTRUCTIONS.split("\n\n").next().unwrap_or_default();
    assert!(first.len() <= 512, "{}", first.len());
}

#[test]
fn exec_shell_task_instructions_name_the_shells_of_exec() {
    let tools = tools();
    let find = |name: &str| {
        tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("{name} missing"))
    };
    let exec = find("exec");
    let description = exec.description.as_deref().unwrap_or_default();
    // Which shell runs where, how Windows PowerShell 5.1 chains commands,
    // how to choose another, and that remote commands do not go through
    // the agent's terminal.
    for text in [INSTRUCTIONS, description] {
        for needle in [
            "PowerShell",
            "pwsh 7",
            "5.1",
            "&&",
            "if ($?)",
            "login shell",
            "`shell`",
            "cmd",
            "`argv`",
            "`se exec`",
        ] {
            assert!(text.contains(needle), "{needle} missing in: {text}");
        }
    }
    // The plugin's skill says the same.
    let skill = include_str!("../plugin/assets/SKILL.md");
    for needle in [
        "PowerShell",
        "pwsh",
        "5.1",
        "&&",
        "if ($?)",
        "login shell",
        "`shell`",
        "`argv`",
        "`se exec`",
        "`os`",
    ] {
        assert!(skill.contains(needle), "{needle} missing in SKILL.md");
    }
    assert!(description.contains("parse error"), "{description}");
    assert!(description.contains("`os`"), "{description}");
    assert_eq!(
        exec.input_schema["properties"]["shell"]["enum"],
        serde_json::json!(["powershell", "cmd", "sh"])
    );
    let sessions = find("sessions").description.as_deref().unwrap_or_default();
    assert!(sessions.contains("`shell`"), "{sessions}");
    let connections = find("connections")
        .description
        .as_deref()
        .unwrap_or_default();
    assert!(connections.contains("`os`"), "{connections}");
    // Agents see the remote-command tools without a tool search.
    for name in ["exec", "sessions", "job_input", "job_cancel", "job_status"] {
        let upfront = find(name)
            .meta
            .as_ref()
            .and_then(|meta| meta.get("anthropic/alwaysLoad"))
            .cloned();
        assert_eq!(upfront, Some(serde_json::Value::Bool(true)), "{name}");
    }
}

/// The working root and the content search (plan 2026-11-27 section C):
/// upfront and read-only, named in the instructions' first paragraph, and
/// the file tools take paths relative to the root.
#[test]
fn mcp_root_task_catalog_offers_root_grep_and_reading_by_lines() {
    let tools = tools();
    let find = |name: &str| {
        tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("{name} missing"))
    };
    let first = INSTRUCTIONS.split("\n\n").next().unwrap_or_default();
    for needle in ["root,", "grep)", "Grep, LS", "grep, cp"] {
        assert!(first.contains(needle), "{needle} missing in: {first}");
    }
    for needle in [
        "`root`",
        "relative",
        "`..` never leaves it",
        "`exec` without target",
        "`grep`",
        "`line`",
    ] {
        assert!(INSTRUCTIONS.contains(needle), "{needle} missing");
    }
    let root = find("root");
    assert_eq!(
        root.input_schema["properties"]["action"]["enum"],
        serde_json::json!(["set", "show", "clear"])
    );
    let grep = find("grep");
    assert_eq!(
        grep.input_schema["required"],
        serde_json::json!(["pattern"])
    );
    assert_eq!(
        grep.input_schema["properties"]["output"]["enum"],
        serde_json::json!(["content", "files", "count"])
    );
    assert_eq!(
        grep.input_schema["properties"]["mode"]["enum"],
        serde_json::json!(["normal", "extended", "regex"])
    );
    for key in [
        "target",
        "pattern",
        "match_case",
        "whole_word",
        "dot_matches_newline",
        "filters",
        "folders",
        "exclude_folders",
        "subfolders",
        "hidden",
        "context_lines",
        "format",
        "max_results",
        "wait_seconds",
        "max_bytes",
    ] {
        assert!(
            grep.input_schema["properties"].get(key).is_some(),
            "grep {key}"
        );
    }
    let read = find("read");
    for key in ["line", "lines"] {
        assert!(
            read.input_schema["properties"].get(key).is_some(),
            "read {key}"
        );
    }
    // A folder target is optional where the working root is a useful default.
    for name in ["list", "stat", "search", "grep", "root"] {
        let required = find(name).input_schema.get("required").cloned();
        assert!(
            !required
                .as_ref()
                .and_then(|value| value.as_array())
                .is_some_and(|items| items.iter().any(|item| item == "target")),
            "{name}: {required:?}"
        );
    }
    for name in ["root", "grep"] {
        let upfront = find(name)
            .meta
            .as_ref()
            .and_then(|meta| meta.get("anthropic/alwaysLoad"))
            .cloned();
        assert_eq!(upfront, Some(serde_json::Value::Bool(true)), "{name}");
    }
    // The working root is named in the target description of every file tool.
    for name in [
        "list", "stat", "read", "write", "edit", "mkdir", "remove", "copy", "move", "search",
        "grep",
    ] {
        let properties = &find(name).input_schema["properties"];
        let description = properties
            .get("target")
            .or_else(|| properties.get("source"))
            .and_then(|value| value["description"].as_str())
            .unwrap_or_default();
        assert!(
            description.contains("working root"),
            "{name}: {description}"
        );
    }
    let skill = include_str!("../plugin/assets/SKILL.md");
    for needle in ["`root`", "`grep`", "`line`"] {
        assert!(skill.contains(needle), "{needle} missing in SKILL.md");
    }
}
