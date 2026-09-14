//! One-time polkit authorization for NetworkManager control, installed with
//! a single `pkexec` prompt. Without it desktop policies may still allow the
//! calls (often `auth_admin_keep`), so the absence of `pkexec` only degrades
//! the automation, never LAN presence.
use std::io;

const RULE_PATH: &str = "/etc/polkit-1/rules.d/49-smart-explorer-lan-uplink.rules";

pub(crate) fn rule_text(user: &str) -> String {
    format!(
        "// Installed by Smart Explorer: lets this user share its internet uplink\n\
         // with paired devices through NetworkManager without a password prompt.\n\
         polkit.addRule(function(action, subject) {{\n\
         \x20   if ((action.id == \"org.freedesktop.NetworkManager.settings.modify.system\" ||\n\
         \x20        action.id == \"org.freedesktop.NetworkManager.network-control\") &&\n\
         \x20       subject.user == \"{user}\") {{\n\
         \x20       return polkit.Result.YES;\n\
         \x20   }}\n\
         }});\n"
    )
}

pub(crate) fn rule_installed() -> bool {
    std::fs::metadata(RULE_PATH).is_ok()
}

fn current_user() -> io::Result<String> {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .map_err(|_| io::Error::other("Benutzername nicht ermittelbar"))?;
    if user.is_empty()
        || !user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(io::Error::other("Benutzername enthaelt ungueltige Zeichen"));
    }
    Ok(user)
}

pub(crate) fn pkexec_available() -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("pkexec").is_file()))
}

/// Install the rule through one `pkexec` prompt.
pub(crate) fn install_rule() -> io::Result<String> {
    if !pkexec_available() {
        return Err(io::Error::other(
            "pkexec ist nicht installiert; polkit-Regel kann nicht automatisch angelegt werden",
        ));
    }
    let user = current_user()?;
    let text = rule_text(&user);
    let staged = crate::support_dirs::app_data_dir().join("lan_uplink");
    std::fs::create_dir_all(&staged)?;
    let staged = staged.join("polkit.rules");
    std::fs::write(&staged, &text)?;
    let status = std::process::Command::new("pkexec")
        .args(["install", "-m", "0644", "-o", "root", "-g", "root"])
        .arg(&staged)
        .arg(RULE_PATH)
        .status();
    let _ = std::fs::remove_file(&staged);
    match status {
        Ok(status) if status.success() => Ok(format!("polkit-Regel installiert: {RULE_PATH}")),
        Ok(status) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("pkexec wurde abgelehnt oder schlug fehl ({status})"),
        )),
        Err(error) => Err(io::Error::other(format!("pkexec starten: {error}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::rule_text;

    #[test]
    fn lan_cleanup_task_rule_names_both_actions_and_the_user() {
        let text = rule_text("alice");
        assert!(text.contains("org.freedesktop.NetworkManager.settings.modify.system"));
        assert!(text.contains("org.freedesktop.NetworkManager.network-control"));
        assert!(text.contains("subject.user == \"alice\""));
        assert!(text.contains("polkit.Result.YES"));
    }
}
