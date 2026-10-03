//! Explicit, atomic root-owned polkit preparation and repair.
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

const RULE_DIR: &str = "/etc/polkit-1/rules.d";
const RULE_PATH: &str = "/etc/polkit-1/rules.d/49-smart-explorer-lan-uplink.rules";
const PKEXEC: &str = "/usr/bin/pkexec";

pub(crate) fn rule_text(user: &str) -> String {
    format!(
        "// Installed by Smart Explorer LAN-Uplink\n\
         polkit.addRule(function(action, subject) {{\n\
         \x20 if ((action.id == \"org.freedesktop.NetworkManager.settings.modify.system\" ||\n\
         \x20      action.id == \"org.freedesktop.NetworkManager.network-control\") &&\n\
         \x20     subject.user == \"{user}\" && subject.local && subject.active) {{\n\
         \x20   return polkit.Result.YES;\n\
         \x20 }}\n\
         }});\n"
    )
}

pub(crate) fn current_user() -> io::Result<String> {
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut found = std::ptr::null_mut();
    let mut buffer = vec![0u8; 64 * 1024];
    let status = unsafe { libc::getpwuid_r(libc::geteuid(), entry.as_mut_ptr(),
        buffer.as_mut_ptr().cast(), buffer.len(), &mut found) };
    if status != 0 || found.is_null() { return Err(io::Error::other("OS-Benutzername nicht ermittelbar")); }
    let name = unsafe { std::ffi::CStr::from_ptr((*found).pw_name) }.to_str()
        .map_err(io::Error::other)?.to_owned();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
        return Err(io::Error::other("OS-Benutzername enthaelt ungueltige Zeichen"));
    }
    Ok(name)
}

/// Missing is safe: NetworkManager may still use the desktop's normal policy.
/// Existing but mismatching, writable or linked rules require explicit repair.
pub(crate) fn rule_status() -> io::Result<bool> {
    match std::fs::symlink_metadata(RULE_PATH) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
        Ok(_) => {},
    }
    for path in ["/etc", "/etc/polkit-1", RULE_DIR] {
        let meta = std::fs::symlink_metadata(path)?;
        if !meta.is_dir() || meta.uid() != 0 || meta.mode() & 0o022 != 0 {
            return Err(io::Error::other("polkit-Regelverzeichnis ist nicht root-geschuetzt"));
        }
    }
    let mut file = std::fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC).open(RULE_PATH)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.uid() != 0 || meta.nlink() != 1 || meta.mode() & 0o777 != 0o644 || meta.len() > 4096 {
        return Err(io::Error::other("polkit-Regel hat unsichere Art, Owner oder Rechte"));
    }
    let mut text = String::new();
    file.by_ref().take(4097).read_to_string(&mut text)?;
    if text != rule_text(&current_user()?) {
        return Err(io::Error::other("polkit-Regel ist veraltet oder gehoert einem anderen Benutzer; Reparatur erforderlich"));
    }
    Ok(true)
}

pub(crate) fn pkexec_available() -> bool { std::path::Path::new(PKEXEC).is_file() }

fn privileged(script: &str) -> io::Result<()> {
    let status = std::process::Command::new(PKEXEC).args(["/bin/sh", "-c", script]).status()?;
    if !status.success() { return Err(io::Error::new(io::ErrorKind::PermissionDenied,
        "polkit-Freigabe abgelehnt oder Vorgang fehlgeschlagen; erneut versuchen")); }
    Ok(())
}

fn directory_guard() -> &'static str {
    "for d in /etc /etc/polkit-1 /etc/polkit-1/rules.d; do test -d \"$d\"; test ! -L \"$d\"; test \"$(/usr/bin/stat -c %u \"$d\")\" = 0; test \"$(/usr/bin/find \"$d\" -maxdepth 0 -perm /022 -print)\" = ''; done; "
}

pub(crate) fn install_rule() -> io::Result<String> {
    let text = rule_text(&current_user()?);
    // No mutable user-staged script/rule crosses the elevation boundary.
    let script = format!("set -eu; umask 077; {}tmp=$(/usr/bin/mktemp '{RULE_DIR}/.se-lan-uplink.XXXXXXXX'); trap '/bin/rm -f -- \"$tmp\"' EXIT; /usr/bin/printf '%s' '{text}' > \"$tmp\"; /bin/chmod 0644 \"$tmp\"; /bin/chown root:root \"$tmp\"; /bin/mv -f -T -- \"$tmp\" '{RULE_PATH}'", directory_guard());
    privileged(&script)?;
    if !rule_status()? { return Err(io::Error::other("polkit-Reparatur wurde nicht bestaetigt")); }
    Ok("Lokale aktive Sitzung fuer NetworkManager eingerichtet".into())
}

pub(crate) fn cleanup() -> io::Result<String> {
    match std::fs::symlink_metadata(RULE_PATH) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok("Keine polkit-Regel vorhanden".into()),
        Err(error) => return Err(error),
        Ok(_) => {},
    }
    privileged(&format!("set -eu; {} /bin/rm -f -- '{RULE_PATH}'", directory_guard()))?;
    match std::fs::symlink_metadata(RULE_PATH) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok("polkit-Regel entfernt".into()),
        _ => Err(io::Error::other("polkit-Regel blieb vorhanden; Entfernung erneut versuchen")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lan_cleanup_task_rule_names_both_actions_and_the_user() {
        let text = super::rule_text("alice");
        assert!(text.contains("org.freedesktop.NetworkManager.settings.modify.system"));
        assert!(text.contains("org.freedesktop.NetworkManager.network-control"));
        assert!(text.contains("subject.user == \"alice\" && subject.local && subject.active"));
        assert!(text.contains("polkit.Result.YES"));
    }
}
