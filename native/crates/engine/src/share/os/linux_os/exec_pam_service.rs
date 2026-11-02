//! PAM service `smart-explorer-exec` of the account units (C3, LC-a):
//! `/etc/pam.d/smart-explorer-exec`. systemd opens a PAM session with it for
//! every account unit (`PAMName=`): the account checks of the distribution
//! (expiry, directory services), `pam_limits`, `pam_loginuid` and
//! `pam_keyinit`, but no `pam_systemd`, so every process of a job stays in
//! its unit and the unit ends them all. `auth` holds only `pam_rootok`:
//! systemd never authenticates on this stack, and nobody but root may use
//! it to authenticate. The distribution family comes from `/etc/os-release`
//! (`ID`, `ID_LIKE`): Debian and Ubuntu, Fedora and RHEL, or any other with
//! local accounts only. A file without the marker line belongs to an
//! administrator and stays as it is.
use std::io::{self, Read};
use std::path::Path;

use super::exec_binary_copy::{opened_trusted, write_root_file};

pub(super) const PAM_SERVICE: &str = "smart-explorer-exec";
pub(super) const PAM_FILE: &str = "/etc/pam.d/smart-explorer-exec";
const MARKER: &str = "# Managed by Smart Explorer";
const MAX_PAM_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PamFamily {
    /// `@include common-account`, `common-session-noninteractive`.
    Debian,
    /// `account include system-auth`.
    Fedora,
    /// Local accounts only (`pam_unix`).
    Other,
}

/// What `ensure_installed` found or did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PamFileState {
    Created,
    Updated,
    Current,
    /// An administrator's own file; it stays as it is.
    Foreign,
}

/// The family of this distribution.
pub(super) fn detect_family() -> PamFamily {
    let text = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    family_of(&text)
}

/// The family `ID` and `ID_LIKE` of an os-release text name.
pub(super) fn family_of(os_release: &str) -> PamFamily {
    let mut names = Vec::new();
    for line in os_release.lines().map(str::trim) {
        let value = line
            .strip_prefix("ID=")
            .or_else(|| line.strip_prefix("ID_LIKE="));
        if let Some(value) = value {
            let value = value.trim_matches(|quote| quote == '"' || quote == '\'');
            names.extend(value.split_whitespace().map(str::to_ascii_lowercase));
        }
    }
    let named = |wanted: [&str; 2]| names.iter().any(|name| wanted.contains(&name.as_str()));
    if named(["debian", "ubuntu"]) {
        PamFamily::Debian
    } else if named(["fedora", "rhel"]) {
        PamFamily::Fedora
    } else {
        PamFamily::Other
    }
}

/// The content of the service file for `family`.
pub(super) fn service_text(family: PamFamily) -> String {
    let rules = match family {
        PamFamily::Debian => {
            "auth     required pam_rootok.so\n\
             @include common-account\n\
             session  required pam_loginuid.so\n\
             session  optional pam_keyinit.so force revoke\n\
             session  required pam_limits.so\n\
             @include common-session-noninteractive\n"
        }
        PamFamily::Fedora => {
            "auth     required pam_rootok.so\n\
             account  include  system-auth\n\
             session  required pam_loginuid.so\n\
             session  optional pam_keyinit.so force revoke\n\
             session  required pam_limits.so\n\
             session  required pam_unix.so\n"
        }
        PamFamily::Other => {
            "auth     required pam_rootok.so\n\
             account  required pam_unix.so\n\
             session  required pam_loginuid.so\n\
             session  optional pam_keyinit.so force revoke\n\
             session  required pam_limits.so\n\
             session  required pam_unix.so\n"
        }
    };
    format!(
        "{MARKER}: PAM service of the units that run remote commands as\n\
         # another account (se share exec accounts). Without pam_systemd every\n\
         # process stays in its unit, which ends them all.\n{rules}"
    )
}

/// Writes the service file for this distribution (root only), unless an
/// administrator manages it.
pub(super) fn ensure_installed() -> io::Result<PamFileState> {
    let expected = service_text(detect_family());
    let path = Path::new(PAM_FILE);
    let current = match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
        Ok(_) => Some(read_service_file(path)?),
    };
    let state = match current {
        None => PamFileState::Created,
        Some(text) if text == expected => return Ok(PamFileState::Current),
        Some(text) if !text.starts_with(MARKER) => return Ok(PamFileState::Foreign),
        Some(_) => PamFileState::Updated,
    };
    write_root_file(path, expected.as_bytes(), 0o644)?;
    Ok(state)
}

/// The account units need the file, and only root may change it (the
/// account service cannot write `/etc`: `ProtectSystem=strict`).
pub(super) fn verify_installed() -> io::Result<()> {
    opened_trusted(Path::new(PAM_FILE))
        .map(drop)
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("PAM service {PAM_FILE} of the account units: {error}"),
            )
        })
}

/// Removes the file where Smart Explorer wrote it (root only).
pub(super) fn remove() -> io::Result<bool> {
    let path = Path::new(PAM_FILE);
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    if !read_service_file(path)?.starts_with(MARKER) {
        return Ok(false);
    }
    std::fs::remove_file(path)?;
    Ok(true)
}

fn read_service_file(path: &Path) -> io::Result<String> {
    let mut text = String::new();
    opened_trusted(path)?
        .take(MAX_PAM_BYTES)
        .read_to_string(&mut text)?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_pam_service_per_family_without_pam_systemd() {
        let ubuntu = "NAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nID=ubuntu\nID_LIKE=debian\n";
        let rocky = "ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n";
        assert_eq!(family_of(ubuntu), PamFamily::Debian);
        assert_eq!(family_of("ID=debian\n"), PamFamily::Debian);
        assert_eq!(family_of("ID=fedora\n"), PamFamily::Fedora);
        assert_eq!(family_of(rocky), PamFamily::Fedora);
        assert_eq!(family_of("ID=arch\n"), PamFamily::Other);
        assert_eq!(family_of(""), PamFamily::Other);
        for family in [PamFamily::Debian, PamFamily::Fedora, PamFamily::Other] {
            let text = service_text(family);
            assert!(text.starts_with(MARKER));
            let rules: Vec<&str> = text.lines().filter(|line| !line.starts_with('#')).collect();
            assert!(
                !rules.iter().any(|line| line.contains("pam_systemd")),
                "{family:?}"
            );
            for module in ["pam_rootok", "pam_loginuid", "pam_limits", "pam_keyinit"] {
                assert!(text.contains(module), "{family:?} lacks {module}");
            }
        }
        assert!(service_text(PamFamily::Debian).contains("@include common-account\n"));
        assert!(service_text(PamFamily::Fedora).contains("account  include  system-auth\n"));
        assert!(service_text(PamFamily::Other).contains("account  required pam_unix.so\n"));
    }
}
