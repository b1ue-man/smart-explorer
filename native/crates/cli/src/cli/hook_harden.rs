//! Unelevated side of the managed hardening (`hook_managed.rs`): computes the
//! wanted content from the mask lists, compares it with the managed files and
//! asks for elevation only when they differ. Called by `se masks` after a
//! change of the protected folders or the enabled state, by `se masks harden`,
//! by the desktop page through `se masks harden`, and by `se plugin
//! uninstall` for the removal.
use super::hook_managed::{self, ManagedRequest};
use serde_json::{json, Value};
use std::path::PathBuf;

/// The program the hooks run: the installed CLI of a managed installation
/// (also when a runtime generation asks), otherwise this `se`.
fn launcher() -> Result<PathBuf, String> {
    let running = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(super::hook_install::hook_program(&running))
}

fn claude_home_present() -> bool {
    crate::support_dirs::home_dir().is_some_and(|home| home.join(".claude").is_dir())
}

fn codex_home_present() -> bool {
    std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::support_dirs::home_dir().map(|home| home.join(".codex")))
        .is_some_and(|home| home.is_dir())
}

/// Local protected folders of the enabled lists (needs the store key).
fn local_scopes() -> Result<Vec<PathBuf>, String> {
    let store = crate::masks::MaskStore::open_default().map_err(|e| e.to_string())?;
    let active = store.active().map_err(|e| e.to_string())?;
    Ok(active.local_scopes())
}

/// Paths the hosts' own file tools may never reach while hardened: the
/// protected local folders plus the se app data (mask store, secret store).
/// The agent host configuration stays out: a deny rule there would also hold
/// while no list is active and take the hosts' own memory and skill files
/// from every session; the lockdown hook and the se tools guard it instead.
fn deny_paths(scopes: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = scopes.to_vec();
    paths.push(crate::support_dirs::app_data_dir());
    paths.retain(|path| path.is_absolute());
    paths.sort();
    paths.dedup();
    paths
}

/// The request for the current lists; `scopes` empty means no list protects
/// a local folder.
pub(in crate::cli) fn wanted() -> Result<(ManagedRequest, usize), String> {
    let scopes = local_scopes()?;
    let request = ManagedRequest {
        launcher: launcher()?,
        deny_paths: deny_paths(&scopes),
        claude: claude_home_present() || hook_managed::claude_dropin().exists(),
        codex: codex_home_present() || hook_managed::codex_requirements().exists(),
    };
    Ok((request, scopes.len()))
}

/// State of the hardening for the surfaces (`se masks status`, desktop page).
pub(in crate::cli) fn status() -> Value {
    match wanted() {
        Ok((request, scopes)) => {
            let mut state = hook_managed::state_for(&request);
            state["protected_local_folders"] = json!(scopes);
            state
        }
        Err(error) => json!({"error": error}),
    }
}

fn up_to_date(state: &Value, request: &ManagedRequest) -> bool {
    (!request.claude || state["claude"]["state"] == "current")
        && (!request.codex || state["codex"]["state"] == "current")
}

/// Brings the managed files to the wanted content. Without a protected local
/// folder and without earlier se entries nothing is written (no dialog);
/// otherwise the system dialog appears once when the content differs.
pub(in crate::cli) fn refresh_if_needed() -> Value {
    let (request, scopes) = match wanted() {
        Ok(wanted) => wanted,
        Err(error) => return json!({"error": error}),
    };
    if scopes == 0 && !hook_managed::present(true, true) {
        return json!({"state":"not_needed","reason":"no enabled mask list protects a local folder"});
    }
    if !request.claude && !request.codex {
        return json!({"state":"not_needed","reason":"neither Claude Code nor Codex was found"});
    }
    let state = hook_managed::state_for(&request);
    if up_to_date(&state, &request) {
        return json!({"state":"current","hosts":state});
    }
    let encoded = match hook_managed::encode_request(&request) {
        Ok(encoded) => encoded,
        Err(error) => return json!({"error": error}),
    };
    match elevated(
        &["apply".into(), encoded],
        "Smart Explorer: Agenten-Sperre der Maskenlisten in Claude Code und Codex verankern",
    ) {
        Ok(()) => json!({"state":"applied","hosts":hook_managed::state_for(&request)}),
        Err(error) => json!({"state":"failed","error":error,"hosts":state}),
    }
}

/// Removes the managed se entries of the hosts (`se plugin uninstall`).
pub(in crate::cli) fn remove(claude: bool, codex: bool) -> Value {
    if !hook_managed::present(claude, codex) {
        return json!({"state":"absent"});
    }
    let host = match (claude, codex) {
        (true, false) => "claude",
        (false, true) => "codex",
        _ => "all",
    };
    match elevated(
        &["remove".into(), host.into()],
        "Smart Explorer: Agenten-Sperre aus den verwalteten Einstellungen entfernen",
    ) {
        Ok(()) => json!({"state":"removed"}),
        Err(error) => json!({"state":"failed","error":error}),
    }
}

/// Asks the user through the system dialog before a change that weakens an
/// active lockdown (unless the lockdown is off).
pub(in crate::cli) fn confirm_if_locked(reason: &str) -> Result<(), String> {
    if !crate::masks::lockdown_state().blocks() {
        return Ok(());
    }
    crate::elevate::confirm_user_presence(reason).map_err(|error| error.to_string())
}

/// Runs `se elevated managed-hooks <args>` with administrator rights: in
/// process when already elevated, otherwise through the system dialog.
fn elevated(args: &[String], reason: &str) -> Result<(), String> {
    if crate::elevate::is_elevated() {
        return hook_managed::execute(args).map(|_| ());
    }
    let program = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut arguments = vec!["elevated".to_string(), "managed-hooks".to_string()];
    arguments.extend(args.iter().cloned());
    let code = super::hook_elevate::run_elevated_program(&program, &arguments, reason)?;
    if code == 0 {
        Ok(())
    } else {
        Err(format!("managed_hooks_elevated_exit: {code}"))
    }
}
