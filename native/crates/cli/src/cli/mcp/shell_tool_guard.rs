//! What the `shell` tool refuses, and the check of the mask store after
//! every command on this device (plan docs/plaene/2026-11-28-ersetzen-
//! masken-shell, S1 hardening K3; contract 1):
//!
//! - Always: commands that start se's own mask administration (`se masks`
//!   other than `list`/`status`/`--help`), the internal helper `se
//!   elevated`, the agent hook `se hook` and removing the agent plugin. The
//!   agent changes lists only through the `masks` tool; the user changes
//!   them at a terminal or in the desktop app.
//! - While a mask list is active: a command, argument, input, environment
//!   value or working directory that names a guarded place (se's app data,
//!   the agent hosts' configuration; `engine::masks::refers_to_guarded`,
//!   `guarded_overlap`).
//! - After every command on this device the integrity guard of the mask
//!   store compares the store with its snapshot; a change from outside is
//!   undone, and this server's `shell` refuses every further command with
//!   the reason ([`session_lock`]).
//!
//! The word check is a hurdle, not the protection: the protection is that
//! every output goes through the mask lists and the store is restored.
use std::sync::Mutex;

use crate::masks::{IntegrityGuard, MaskSet};
use crate::share::ExecCommand;

/// Why this server's shell is locked, once the store was changed from
/// outside (one `se mcp` process serves one agent session).
static LOCKED: Mutex<Option<String>> = Mutex::new(None);

/// The reason this session's commands are refused, if they are.
pub(super) fn session_lock() -> Option<String> {
    LOCKED.lock().ok().and_then(|slot| slot.clone())
}

fn lock_session(reason: String) {
    if let Ok(mut slot) = LOCKED.lock() {
        slot.get_or_insert(reason);
    }
}

/// Everything a command call hands to the shell.
pub(super) struct CallTexts<'a> {
    pub(super) command: &'a ExecCommand,
    pub(super) cwd: Option<&'a str>,
    pub(super) env: &'a std::collections::BTreeMap<String, String>,
    pub(super) stdin: Option<&'a str>,
}

/// Why the call is refused, if it is.
pub(super) fn refusal(set: &MaskSet, call: &CallTexts<'_>) -> Option<String> {
    let words = command_words(call.command);
    if let Some(reason) = administration(&words) {
        return Some(reason);
    }
    if set.is_empty() {
        return None;
    }
    let mut texts: Vec<&str> = words.iter().map(String::as_str).collect();
    texts.extend(call.cwd);
    texts.extend(call.stdin);
    texts.extend(
        call.env
            .iter()
            .flat_map(|(name, value)| [name.as_str(), value.as_str()]),
    );
    for text in texts {
        if let Some(place) = crate::masks::refers_to_guarded(text) {
            return Some(format!(
                "this command names Smart Explorer's own data or an agent host's configuration ({}), which se leaves alone while a mask list is active",
                place.display()
            ));
        }
    }
    if let Some(cwd) = call.cwd {
        if let Some(place) = crate::masks::guarded_overlap(std::path::Path::new(cwd)) {
            return Some(format!(
                "the working directory {cwd} touches Smart Explorer's own data or an agent host's configuration ({})",
                place.display()
            ));
        }
    }
    None
}

/// The words of a command: the shell line split at blanks and separators,
/// or the program and its arguments.
fn command_words(command: &ExecCommand) -> Vec<String> {
    match command {
        ExecCommand::Argv { program, args } => std::iter::once(program.clone())
            .chain(args.iter().cloned())
            .collect(),
        ExecCommand::Shell { command } => command
            .split(|ch: char| ch.is_whitespace() || ";&|()`{}<>".contains(ch))
            .map(|word| word.trim_matches(['"', '\'']).to_string())
            .filter(|word| !word.is_empty())
            .collect(),
        ExecCommand::Login => Vec::new(),
    }
}

/// A call of se's administration: `se` (any path, `.exe`) followed, after
/// options, by `masks` (but `list`, `status`, `--help`), `elevated`, `hook`
/// or `plugin uninstall`.
fn administration(words: &[String]) -> Option<String> {
    for (at, word) in words.iter().enumerate() {
        let program = word.rsplit(['/', '\\']).next().unwrap_or(word);
        let program = program.strip_suffix(".exe").unwrap_or(program);
        if !program.eq_ignore_ascii_case("se") {
            continue;
        }
        let rest: Vec<&str> = words[at + 1..]
            .iter()
            .map(String::as_str)
            .filter(|word| !word.starts_with('-') || *word == "--help")
            .take(2)
            .collect();
        match rest.as_slice() {
            ["masks", "list" | "status" | "--help", ..] | ["masks", "--help"] => {}
            ["masks", ..] => {
                return Some(
                    "`se masks` is for the user at a terminal or in the desktop app; use the `masks` tool to list lists and add lists, entries and guarded folders".into(),
                )
            }
            ["elevated", ..] => {
                return Some(
                    "`se elevated` is an internal helper of se; give `elevated: true` to run a command with administrator or root rights".into(),
                )
            }
            ["hook", ..] => return Some("`se hook` is the agent host's hook, not a command".into()),
            ["plugin", "uninstall" | "retire-installation"] => {
                return Some(
                    "removing the agent plugin is for the user; run `se plugin uninstall` in a terminal".into(),
                )
            }
            _ => {}
        }
    }
    None
}

/// The guard to check after a command on this device: `Ok(None)` when there
/// is no mask list at all; an unreadable store with active lists refuses
/// the command (nothing good to restore).
pub(super) fn take_guard(
    layer: &super::mask_layer::MaskLayer,
    set: &MaskSet,
) -> Result<Option<IntegrityGuard>, String> {
    match layer.store(|store| store.guard().map_err(|error| error.to_string())) {
        Ok(guard) => Ok(Some(guard)),
        Err(_) if set.is_empty() => Ok(None),
        Err(error) => Err(format!(
            "the mask list store cannot be checked, so no command runs while a list is active: {error}"
        )),
    }
}

/// After a command: a store changed from outside is restored and the
/// session locked; the text for the job's result then.
pub(super) fn check_after(guard: Option<&mut IntegrityGuard>) -> Option<String> {
    let guard = guard?;
    match guard.check_and_restore() {
        Ok(()) => None,
        Err(tampered) => {
            let reason = format!(
                "{tampered}; this agent session runs no further commands (start a new session after the user checked the mask lists)"
            );
            lock_session(reason.clone());
            Some(reason)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(command: &str) -> ExecCommand {
        ExecCommand::Shell {
            command: command.into(),
        }
    }

    #[test]
    fn shell_elevate_task_mask_administration_is_refused_in_every_form() {
        for refused in [
            "se masks unlock Kunden",
            "cd /tmp && /usr/local/bin/se --json masks remove-entry x",
            "SE.EXE masks disable",
            "echo hi; se elevated exec -- id",
            "(se hook claude)",
            "se plugin uninstall",
        ] {
            assert!(
                administration(&command_words(&shell(refused))).is_some(),
                "{refused}"
            );
        }
        for allowed in [
            "se masks list",
            "se masks --help",
            "se ls /tmp",
            "git status",
            "sed -n 1p x",
        ] {
            assert!(
                administration(&command_words(&shell(allowed))).is_none(),
                "{allowed}"
            );
        }
        let argv = ExecCommand::Argv {
            program: r"C:\Tools\se.exe".into(),
            args: vec!["masks".into(), "lock".into()],
        };
        assert!(administration(&command_words(&argv)).is_some());
        let set = MaskSet::empty();
        let env = std::collections::BTreeMap::new();
        let call = CallTexts {
            command: &shell("ls ~/.claude"),
            cwd: None,
            env: &env,
            stdin: None,
        };
        // Without an active list nothing but the administration is refused.
        assert!(refusal(&set, &call).is_none());
    }
}
