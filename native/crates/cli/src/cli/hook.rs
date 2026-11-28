//! `se hook <claude|codex>`: the `PreToolUse` (and Claude Code
//! `PreSkillShellCommand`) hook of the agent hosts. It decides one tool call
//! from stdin within milliseconds: while no active mask list has a protected
//! folder it prints nothing (the host's own permission flow continues);
//! otherwise, or while the mask state cannot be read, it refuses every tool
//! except the se tools and tools without file or process access
//! (`hook_policy.rs`). Output formats: Claude Code and Codex read
//! `hookSpecificOutput.permissionDecision = "deny"` with a non-empty reason
//! (docs/refs/agenten-sperre-elevation-2026-11-28.md §1.3, §2.1); exit 2 with
//! the reason on stderr is the fallback when stdout cannot be written.
#[path = "hook_elevate.rs"]
mod hook_elevate;
#[path = "hook_harden.rs"]
pub(super) mod hook_harden;
#[path = "hook_install.rs"]
pub(super) mod hook_install;
#[path = "hook_managed.rs"]
pub(super) mod hook_managed;
#[cfg(test)]
#[path = "hook_managed_tests.rs"]
mod hook_managed_tests;
#[path = "hook_policy.rs"]
mod hook_policy;
#[path = "hook_servers.rs"]
pub(super) mod hook_servers;
#[cfg(test)]
#[path = "hook_tests.rs"]
mod hook_tests;
#[path = "hook_web.rs"]
mod hook_web;

use hook_policy::{Decision, HookHost};
use serde_json::{json, Value};
use std::io::{Read, Write};

/// Largest hook input read (a `Write` call carries the whole file content).
const MAX_INPUT: u64 = 256 * 1024 * 1024;

#[derive(clap::Args)]
pub(super) struct HookArgs {
    /// Host whose hook input arrives on stdin.
    #[arg(value_enum)]
    host: HookHost,
}

/// Lockdown as the hook sees it (from `crate::masks`, no key needed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Lockdown {
    Off,
    On,
    Unknown,
}

pub(super) fn run(args: HookArgs) -> Result<i32, String> {
    let mut input = Vec::new();
    let read = std::io::stdin()
        .lock()
        .take(MAX_INPUT + 1)
        .read_to_end(&mut input);
    let lockdown = current_lockdown();
    let decision = if lockdown == Lockdown::Off {
        Decision::Allow
    } else {
        match read {
            Ok(_) if input.len() as u64 <= MAX_INPUT => {
                decide_bytes(args.host, lockdown, &input, &allowed_servers())
            }
            Ok(_) => Decision::Deny(
                "Smart Explorer agent lockdown: the tool call is too large to check; it is blocked."
                    .into(),
            ),
            Err(error) => Decision::Deny(format!(
                "Smart Explorer agent lockdown: the tool call could not be read ({error}); it is blocked."
            )),
        }
    };
    let event = serde_json::from_slice::<Value>(&input)
        .ok()
        .and_then(|value| value["hook_event_name"].as_str().map(str::to_owned));
    Ok(emit(&decision, event.as_deref()))
}

/// Decision for raw hook input bytes; separate from stdin for the tests.
fn decide_bytes(
    host: HookHost,
    lockdown: Lockdown,
    input: &[u8],
    allowed_servers: &[String],
) -> Decision {
    if lockdown == Lockdown::Off {
        return Decision::Allow;
    }
    let Ok(value) = serde_json::from_slice::<Value>(input) else {
        return Decision::Deny(
            "Smart Explorer agent lockdown: the hook input is not JSON; the tool call is blocked."
                .into(),
        );
    };
    let decision = match value["hook_event_name"].as_str() {
        Some("PreSkillShellCommand") => Decision::Deny(hook_policy::skill_shell_reason()),
        // Events this hook is not registered for are never decided here.
        Some(event) if event != "PreToolUse" => Decision::Allow,
        _ => hook_policy::decide_tool(host, &value, allowed_servers),
    };
    match (decision, lockdown) {
        (Decision::Deny(reason), Lockdown::Unknown) => {
            Decision::Deny(format!("{reason} {}", hook_policy::unknown_state_reason()))
        }
        (decision, _) => decision,
    }
}

/// Writes the decision; returns the exit status.
fn emit(decision: &Decision, event: Option<&str>) -> i32 {
    let Decision::Deny(reason) = decision else {
        return 0;
    };
    // `PreSkillShellCommand` documents only exit 2 as its block.
    if event == Some("PreSkillShellCommand") {
        eprintln!("{reason}");
        return 2;
    }
    let output = deny_output(reason);
    let mut stdout = std::io::stdout().lock();
    if writeln!(stdout, "{output}")
        .and_then(|()| stdout.flush())
        .is_ok()
    {
        0
    } else {
        eprintln!("{reason}");
        2
    }
}

/// The `PreToolUse` refusal both hosts read.
fn deny_output(reason: &str) -> Value {
    json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":reason}})
}

/// Lockdown from the mask store state: `On` while an active list has a
/// protected folder; `Unknown` (fail-closed) when the state exists but cannot
/// be read.
fn current_lockdown() -> Lockdown {
    match crate::masks::lockdown_state() {
        crate::masks::LockdownState::Off => Lockdown::Off,
        crate::masks::LockdownState::On => Lockdown::On,
        crate::masks::LockdownState::Unknown => Lockdown::Unknown,
    }
}

/// Foreign MCP servers the user allowed (`se masks servers`), readable
/// without the store key.
fn allowed_servers() -> Vec<String> {
    hook_servers::allowed()
}
