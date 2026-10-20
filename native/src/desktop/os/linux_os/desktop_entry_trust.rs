//! Whether se may start a `.desktop` file on the desktop itself (U34-6,
//! C2.7). A starter is code the user's desktop runs with their rights, so
//! se follows the rule of the running desktop and never makes a file trusted
//! (`metadata::trusted` is not set by se):
//!
//! - GNOME and Ubuntu: the file is executable, `gio info -a metadata::trusted`
//!   says `true`, and neither the file nor the desktop folder is writable for
//!   the group or others;
//! - KDE: the file is executable or owned by root
//!   (`KDesktopFile::isAuthorizedDesktopFile`);
//! - Xfce: `metadata::xfce-exe-checksum` is the SHA-256 of the file;
//! - every other desktop, and a rule that is not met: the OpenURI portal
//!   with a choice (`open_with_portal`).
//!
//! `Direct` starts `gio launch <file>`; without `gio` the answer is `Portal`.
use std::path::Path;

use sha2::{Digest, Sha256};

use super::gnome_settings::run_command;

/// How a starter is opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Launch {
    Direct,
    Portal,
}

/// The desktop whose rule applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DesktopKind {
    Gnome,
    Kde,
    Xfce,
    Other,
}

/// The rule for `XDG_CURRENT_DESKTOP` (`ubuntu:GNOME`, `KDE`, `XFCE`).
pub(crate) fn desktop_kind(current_desktop: &str) -> DesktopKind {
    let has = |name: &str| {
        current_desktop
            .split(':')
            .any(|part| part.trim().eq_ignore_ascii_case(name))
    };
    if has("GNOME") || has("Unity") {
        DesktopKind::Gnome
    } else if has("KDE") {
        DesktopKind::Kde
    } else if has("XFCE") {
        DesktopKind::Xfce
    } else {
        DesktopKind::Other
    }
}

/// The facts the rules ask for.
pub(crate) trait TrustProbe {
    /// The permission bits (`st_mode & 0o7777`) and the owner of `path`.
    fn mode_and_owner(&self, path: &Path) -> Option<(u32, u32)>;
    /// The current user may execute `path`.
    fn executable(&self, path: &Path) -> bool;
    /// `gio info -a <attribute>`: the value of the attribute.
    fn gio_attribute(&self, path: &Path, attribute: &str) -> Option<String>;
    fn read(&self, path: &Path) -> Option<Vec<u8>>;
    fn gio_available(&self) -> bool;
}

/// The value of `attribute` in the output of `gio info -a <attribute>`.
pub(crate) fn parse_gio_attribute(output: &str, attribute: &str) -> Option<String> {
    let prefix = format!("{attribute}:");
    output
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(&prefix))
        .map(|value| value.trim().to_string())
}

fn not_writable_by_others(probe: &dyn TrustProbe, path: &Path) -> bool {
    probe
        .mode_and_owner(path)
        .is_some_and(|(mode, _)| mode & 0o022 == 0)
}

/// How `path` is opened on `desktop`.
pub(crate) fn launch_decision(
    path: &Path,
    desktop: &DesktopKind,
    probe: &dyn TrustProbe,
) -> Launch {
    if !probe.gio_available() {
        return Launch::Portal;
    }
    let trusted = match desktop {
        DesktopKind::Gnome => {
            let folder_safe = path
                .parent()
                .is_some_and(|folder| not_writable_by_others(probe, folder));
            probe.executable(path)
                && probe
                    .gio_attribute(path, "metadata::trusted")
                    .is_some_and(|value| value == "true")
                && not_writable_by_others(probe, path)
                && folder_safe
        }
        DesktopKind::Kde => {
            probe.executable(path)
                || probe
                    .mode_and_owner(path)
                    .is_some_and(|(_, owner)| owner == 0)
        }
        DesktopKind::Xfce => {
            let sum = probe.gio_attribute(path, "metadata::xfce-exe-checksum");
            let content = probe.read(path);
            matches!((sum, content), (Some(sum), Some(content))
                if sum.eq_ignore_ascii_case(&hex_sha256(&content)))
        }
        DesktopKind::Other => false,
    };
    if trusted {
        Launch::Direct
    } else {
        Launch::Portal
    }
}

pub(crate) fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The real files and `gio`.
pub(crate) struct SystemTrust;

impl TrustProbe for SystemTrust {
    fn mode_and_owner(&self, path: &Path) -> Option<(u32, u32)> {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path).ok()?;
        Some((meta.mode() & 0o7777, meta.uid()))
    }

    fn executable(&self, path: &Path) -> bool {
        use std::os::unix::ffi::OsStrExt;
        let Ok(text) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return false;
        };
        // SAFETY: `text` is a NUL-terminated path that lives through the call.
        unsafe { libc::access(text.as_ptr(), libc::X_OK) == 0 }
    }

    fn gio_attribute(&self, path: &Path, attribute: &str) -> Option<String> {
        let output = run_command(
            Path::new("gio"),
            &["info", "-a", attribute, &path.to_string_lossy()],
        )?;
        parse_gio_attribute(&output, attribute)
    }

    fn read(&self, path: &Path) -> Option<Vec<u8>> {
        std::fs::read(path).ok()
    }

    fn gio_available(&self) -> bool {
        std::env::var_os("PATH")
            .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join("gio").is_file()))
    }
}

/// `gio launch <file>` with no stream of ours; the child is collected on a
/// thread. `false` when `gio` did not start.
pub(crate) fn launch_direct(path: &Path) -> bool {
    let spawned = std::process::Command::new("gio")
        .arg("launch")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = spawned else {
        return false;
    };
    let _ = std::thread::Builder::new()
        .name("gio-launch-reaper".into())
        .spawn(move || {
            let _ = child.wait();
        });
    true
}
