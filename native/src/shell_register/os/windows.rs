// Per-user Windows shell integration (no admin), all reversible:
//   1. "In Smart Explorer öffnen" right-click verb on folders/drives/background.
//   2. Opt-in: make Smart Explorer the default handler for opening folders and
//      drives (what double-clicking a folder launches).
//
// Everything is written under HKCU\Software\Classes (which the shell merges
// over HKLM with user priority). We never touch HKLM/HKCR or the Folder class.
//
// Default-manager mechanism (verified on Win11): the built-in folder "open"
// verb lives on the Folder base class with a DelegateExecute COM handler that
// routes to Explorer. Directory/Drive derive from Folder and are resolved
// first, so creating a SELF-CONTAINED Directory\shell\open\command (with only
// a (Default) value — no DelegateExecute, no ddeexec) shadows it and runs our
// exe. Drive is a separate class and must be written too.
//
// Reversal: before enabling we record whether each ...\shell\open key already
// existed in HKCU (the common clean-machine case: it did NOT). "Disable" then
// deletes exactly the keys we created, letting the inherited system default
// resurface — we never write Explorer's path back. If a key DID pre-exist
// (another tool owned it), we restore its prior command verbatim instead.

#![cfg(windows)]

use std::io;
use winreg::enums::*;
use winreg::RegKey;

const VERB: &str = "OpenInSmartExplorer";
const LABEL: &str = "In Smart Explorer öffnen";

fn exe_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().replace('/', "\\"))
        .unwrap_or_default()
}

/// Command string for the registry: `"<exe>" "<arg>"` where arg is %1 (the
/// clicked item) or %V (the folder's own path, for the Background context).
fn command_for(arg: &str) -> String {
    format!("\"{}\" \"{}\"", exe_path(), arg)
}

fn icon_value() -> String {
    format!("\"{}\",0", exe_path())
}

fn backup_path() -> std::path::PathBuf {
    let dir = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("smart_explorer");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("default_manager_backup.txt")
}

fn is_not_found(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::NotFound
}

// ─── Context-menu verb (benign, additive) ───────────────────────────────────

fn register_verb(class_path: &str, arg: &str) -> io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (verb, _) =
        hkcu.create_subkey(format!(r"Software\Classes\{}\shell\{}", class_path, VERB))?;
    verb.set_value("MUIVerb", &LABEL)?;
    verb.set_value("Icon", &icon_value())?;
    let (cmd, _) = hkcu.create_subkey(format!(
        r"Software\Classes\{}\shell\{}\command",
        class_path, VERB
    ))?;
    cmd.set_value("", &command_for(arg))?;
    Ok(())
}

fn unregister_verb(class_path: &str) -> io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    match hkcu.delete_subkey_all(format!(r"Software\Classes\{}\shell\{}", class_path, VERB)) {
        Ok(()) => Ok(()),
        Err(e) if is_not_found(&e) => Ok(()), // already gone — idempotent
        Err(e) => Err(e),
    }
}

/// Folders & drives use %1 (the clicked item); the Background context (empty
/// space inside an open window) has no clicked item, so it uses %V.
pub fn set_context_menu(on: bool) -> io::Result<()> {
    if on {
        register_verb("Directory", "%1")?;
        register_verb("Drive", "%1")?;
        register_verb(r"Directory\Background", "%V")?;
    } else {
        unregister_verb("Directory")?;
        unregister_verb("Drive")?;
        unregister_verb(r"Directory\Background")?;
    }
    notify_assoc_changed();
    Ok(())
}

pub fn context_menu_enabled() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(format!(
            r"Software\Classes\Directory\shell\{}\command",
            VERB
        ))
        .is_ok()
}

// ─── Default file manager (opt-in, fully reversible) ─────────────────────────

const CLASSES: [&str; 2] = ["Directory", "Drive"];

/// Snapshot whether each `...\shell\open` key already existed in HKCU and, if
/// so, its command's prior (Default) — persisted before we write anything so
/// "disable" (and uninstall) can restore precisely even after a restart.
fn capture_backup() {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let mut lines: Vec<String> = Vec::new();
    for cls in CLASSES {
        let open_path = format!(r"Software\Classes\{}\shell\open", cls);
        let existed = hkcu.open_subkey(&open_path).is_ok();
        let prior_cmd = hkcu
            .open_subkey(format!(r"{}\command", open_path))
            .ok()
            .and_then(|k| k.get_value::<String, _>("").ok())
            .unwrap_or_default();
        let key = cls.to_lowercase();
        lines.push(format!("{}_existed={}", key, if existed { 1 } else { 0 }));
        // Single line, no embedded newlines expected in a command string.
        lines.push(format!("{}_command={}", key, prior_cmd));
    }
    let _ = std::fs::write(backup_path(), lines.join("\n"));
}

fn read_backup() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if let Ok(txt) = std::fs::read_to_string(backup_path()) {
        for line in txt.lines() {
            if let Some((k, v)) = line.split_once('=') {
                map.insert(k.to_string(), v.to_string());
            }
        }
    }
    map
}

// The default-manager override doesn't work on Win11 (kept only to exercise the
// reversibility test and the cleanup path).
#[cfg_attr(not(test), allow(dead_code))]
fn enable_default_manager() -> io::Result<()> {
    capture_backup(); // MUST happen before the first write
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let command = command_for("%1");
    for cls in CLASSES {
        let (cmd, _) =
            hkcu.create_subkey(format!(r"Software\Classes\{}\shell\open\command", cls))?;
        cmd.set_value("", &command)?;
        // A freshly created key has neither, but if a prior tool left a
        // DelegateExecute value or ddeexec subkey on the open verb it would
        // re-route to Explorer — strip them so our command actually runs.
        if let Ok(open) = hkcu.open_subkey_with_flags(
            format!(r"Software\Classes\{}\shell\open", cls),
            KEY_ALL_ACCESS,
        ) {
            let _ = open.delete_value("DelegateExecute");
            let _ = open.delete_subkey_all("ddeexec");
        }
    }
    notify_assoc_changed();
    Ok(())
}

fn disable_default_manager() -> io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let bk = read_backup();
    for cls in CLASSES {
        let key = cls.to_lowercase();
        let existed = bk
            .get(&format!("{}_existed", key))
            .map(|s| s == "1")
            .unwrap_or(false);
        let prior_cmd = bk
            .get(&format!("{}_command", key))
            .cloned()
            .unwrap_or_default();
        if existed && !prior_cmd.is_empty() {
            // CASE 2 (rare): another handler owned this verb — restore it verbatim.
            if let Ok((cmd, _)) =
                hkcu.create_subkey(format!(r"Software\Classes\{}\shell\open\command", cls))
            {
                let _ = cmd.set_value("", &prior_cmd);
            }
        } else {
            // CASE 1 (clean machine): delete exactly the subtree we created;
            // the inherited Folder/Explorer default then resurfaces.
            match hkcu.delete_subkey_all(format!(r"Software\Classes\{}\shell\open", cls)) {
                Ok(()) => {}
                Err(e) if is_not_found(&e) => {}
                Err(_) => {}
            }
        }
    }
    let _ = std::fs::remove_file(backup_path());
    notify_assoc_changed();
    Ok(())
}

/// True if OUR exe is the current Directory open handler.
fn default_manager_enabled() -> bool {
    let exe = exe_path().to_lowercase();
    if let Ok(k) = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Classes\Directory\shell\open\command")
    {
        if let Ok(v) = k.get_value::<String, _>("") {
            let v = v.to_lowercase();
            return v.contains("smart explorer.exe") || (!exe.is_empty() && v.contains(&exe));
        }
    }
    false
}

/// Remove every integration we may have added. Called by the uninstaller (via
/// the `--unregister` flag) so an uninstall never leaves folder-opening pointed
/// at a deleted exe.
pub fn unregister_all() {
    let _ = set_context_menu(false);
    let _ = disable_default_manager();
}

/// Self-heal for the default-manager toggle that shipped in 0.3.4 but never
/// worked on Win11 (the registry override doesn't redirect folder activation).
/// On startup we remove any override keys / backup it left behind, so no user
/// is stuck with a dangling, useless folder handler.
pub fn cleanup_stale_default_manager() {
    if default_manager_enabled() || backup_path().exists() {
        let _ = disable_default_manager();
    }
}

fn notify_assoc_changed() {
    use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Proves the default-manager toggle is fully reversible on the LIVE
    // registry: enable writes our keys, disable returns to the exact prior
    // state (key absent on a clean machine) and never clobbers sibling verbs.
    #[test]
    fn default_manager_is_reversible() {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let dir_open = r"Software\Classes\Directory\shell\open";
        let drv_open = r"Software\Classes\Drive\shell\open";

        // Only exercise the destructive path on a clean (CASE 1) machine, so an
        // automated run never disturbs a real pre-existing folder handler.
        if hkcu.open_subkey(dir_open).is_ok() || hkcu.open_subkey(drv_open).is_ok() {
            eprintln!("skip: a folder-open verb already exists in HKCU (CASE 2)");
            return;
        }
        // Record sibling verbs that must survive untouched.
        let sibling = r"Software\Classes\Directory\shell\WizTree";
        let sibling_before = hkcu.open_subkey(sibling).is_ok();

        enable_default_manager().expect("enable");
        assert!(
            hkcu.open_subkey(format!(r"{}\command", dir_open)).is_ok(),
            "Directory open command should exist after enable"
        );
        assert!(
            hkcu.open_subkey(format!(r"{}\command", drv_open)).is_ok(),
            "Drive open command should exist after enable"
        );

        disable_default_manager().expect("disable");
        assert!(
            hkcu.open_subkey(dir_open).is_err(),
            "Directory open verb must be gone after disable (clean reversal)"
        );
        assert!(
            hkcu.open_subkey(drv_open).is_err(),
            "Drive open verb must be gone after disable (clean reversal)"
        );
        assert_eq!(
            hkcu.open_subkey(sibling).is_ok(),
            sibling_before,
            "sibling verb (WizTree) must be untouched by our enable/disable"
        );
    }

    #[test]
    fn context_menu_is_reversible() {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        set_context_menu(true).expect("enable ctx menu");
        assert!(
            context_menu_enabled(),
            "verb should be present after enable"
        );
        assert!(hkcu
            .open_subkey(r"Software\Classes\Directory\Background\shell\OpenInSmartExplorer\command")
            .is_ok());

        set_context_menu(false).expect("disable ctx menu");
        assert!(!context_menu_enabled(), "verb should be gone after disable");
        assert!(hkcu
            .open_subkey(r"Software\Classes\Drive\shell\OpenInSmartExplorer")
            .is_err());
    }
}
