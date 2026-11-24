//! Whether this Linux system can serve FUSE drives (DRV-M4, `se drive
//! runtime`, decision C2 "Grenzen, ehrlich angezeigt").
//!
//! Ready means: `/dev/fuse` is readable and writable for this user, and a
//! `fusermount3` that is setuid root exists (no `user_allow_other` is
//! needed). The report also names the kernel request timeouts
//! (`fs.fuse.default_request_timeout`, `max_request_timeout`; a value above
//! 0 lets the kernel abort a drive whose request takes longer, se does not
//! work around it), a sandbox (Flatpak, Snap: not supported, assumption A9;
//! container), WSL (the Windows Explorer sees FUSE mounts only below
//! `/mnt/wsl`, assumption A7).
//! Every path is read below an exchangeable root and the environment comes
//! from a function, so the detection is testable without the real system.
//!
//! A missing `fusermount3` or `/dev/fuse` is installed on use (user order
//! 2026-11-24): the drive host, `se drive install-runtime` and the window's
//! state go through `login/os/linux_os/package_install.rs` (package fuse3,
//! the kernel module `fuse`), behind the polkit dialog, and then check once
//! more. A sandbox (Flatpak, Snap) and a helper without setuid are no case
//! for an install; their problem is reported as it is.
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::login::os::platform::package_install::{self, Component};
use crate::login::os::shared::install_gate::{with_install, Attempt, Failure};

const STANDARD_DIRS: [&str; 5] = ["/usr/bin", "/bin", "/usr/local/bin", "/usr/sbin", "/sbin"];
const HELPER: &str = "fusermount3";

/// Where the facts come from: `/` and the process environment in
/// production, a fixture tree in tests.
pub struct RuntimeProbe<'a> {
    pub root: &'a Path,
    pub env: &'a dyn Fn(&str) -> Option<OsString>,
    /// Ask `fusermount3 -V` for its version (spawns a process).
    pub version: bool,
}

impl RuntimeProbe<'static> {
    pub fn system(version: bool) -> Self {
        Self {
            root: Path::new("/"),
            env: &system_env,
            version,
        }
    }
}

fn system_env(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Sandbox {
    None,
    Flatpak,
    Snap,
    Container,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FuseDevice {
    pub path: String,
    pub accessible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FuseHelper {
    pub path: Option<String>,
    pub setuid: bool,
    pub version: Option<String>,
}

/// JSON of `se drive runtime --json` under Linux (contract of DRV-M4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FuseRuntimeReport {
    pub platform: &'static str,
    pub ready: bool,
    pub device: FuseDevice,
    pub fusermount3: FuseHelper,
    pub kernel_request_timeout_s: Option<u64>,
    pub kernel_max_request_timeout_s: Option<u64>,
    pub sandbox: Sandbox,
    pub wsl: bool,
    pub problems: Vec<String>,
}

/// `se drive install-runtime` under Linux: se installs fuse3 (and loads the
/// kernel module) behind the polkit dialog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FuseInstallOutcome {
    AlreadyReady,
    Installed,
    /// The polkit dialog was dismissed or refused.
    Declined {
        reason: String,
    },
    /// The install failed, or FUSE is not ready for a reason an install
    /// cannot change; the exact reason.
    Failed {
        reason: String,
    },
}

impl FuseInstallOutcome {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::AlreadyReady | Self::Installed => 0,
            Self::Failed { .. } => 1,
            Self::Declined { .. } => 2,
        }
    }
}

pub fn report(probe: &RuntimeProbe<'_>) -> FuseRuntimeReport {
    let device_path = probe.root.join("dev/fuse");
    let device_exists = device_path.exists();
    let device = FuseDevice {
        path: "/dev/fuse".into(),
        accessible: device_exists && read_write(&device_path),
    };
    let helper = find_helper(probe);
    let setuid = helper
        .as_deref()
        .is_some_and(|path| setuid_root(&below(probe.root, path)));
    let version = match (&helper, probe.version) {
        (Some(path), true) => helper_version(path),
        _ => None,
    };
    let sandbox = sandbox(probe);
    let mut problems = Vec::new();
    if !device_exists {
        problems.push(
            "/dev/fuse fehlt: das FUSE-Kernelmodul ist nicht geladen oder die Umgebung gibt das Gerät nicht frei"
                .to_string(),
        );
    } else if !device.accessible {
        problems.push("/dev/fuse ist für diesen Nutzer nicht les- und schreibbar".to_string());
    }
    match &helper {
        None => problems.push("fusermount3 fehlt (Paket fuse3)".to_string()),
        Some(path) if !setuid => problems.push(format!(
            "{} ist nicht setuid root; Laufwerke brauchen den Setuid-Helfer aus dem Paket fuse3",
            path.display()
        )),
        Some(_) => {}
    }
    match sandbox {
        Sandbox::Flatpak => problems.push(
            "Flatpak-Sandkasten: Smart-Explorer-Laufwerke werden dort nicht unterstützt"
                .to_string(),
        ),
        Sandbox::Snap => problems.push(
            "Snap-Sandkasten: Smart-Explorer-Laufwerke werden dort nicht unterstützt".to_string(),
        ),
        Sandbox::None | Sandbox::Container => {}
    }
    FuseRuntimeReport {
        platform: "linux",
        ready: problems.is_empty(),
        device,
        fusermount3: FuseHelper {
            path: helper.map(|path| path.display().to_string()),
            setuid,
            version,
        },
        kernel_request_timeout_s: sysctl(probe.root, "default_request_timeout"),
        kernel_max_request_timeout_s: sysctl(probe.root, "max_request_timeout"),
        sandbox,
        wsl: wsl(probe),
        problems,
    }
}

/// Whether installing fuse3 (and loading the module) can make FUSE ready:
/// the helper or the device is missing and no sandbox forbids drives.
pub fn install_helps(report: &FuseRuntimeReport, device_present: bool) -> bool {
    !matches!(report.sandbox, Sandbox::Flatpak | Sandbox::Snap)
        && (report.fusermount3.path.is_none() || !device_present)
}

fn device_present(probe: &RuntimeProbe<'_>) -> bool {
    probe.root.join("dev/fuse").exists()
}

/// FUSE ready, `Missing` when an install helps, else the problems.
fn readiness(probe: &RuntimeProbe<'_>) -> Result<(), Failure> {
    let report = report(probe);
    if report.ready {
        Ok(())
    } else if install_helps(&report, device_present(probe)) {
        Err(Failure::Missing(report.problems.join("; ")))
    } else {
        Err(Failure::Other(report.problems.join("; ")))
    }
}

/// `se drive install-runtime` with `install` as the package step (the
/// tests pass their own).
pub fn install_with(
    probe: &RuntimeProbe<'_>,
    install: impl FnOnce() -> Attempt,
) -> FuseInstallOutcome {
    match readiness(probe) {
        Ok(()) => return FuseInstallOutcome::AlreadyReady,
        Err(Failure::Other(reason)) => return FuseInstallOutcome::Failed { reason },
        Err(Failure::Missing(_)) => {}
    }
    match install() {
        Attempt::Installed => match readiness(probe) {
            Ok(()) => FuseInstallOutcome::Installed,
            Err(failure) => FuseInstallOutcome::Failed {
                reason: format!(
                    "FUSE ist nach der Installation nicht bereit: {}",
                    failure.into_reason()
                ),
            },
        },
        Attempt::Declined(reason) => FuseInstallOutcome::Declined { reason },
        Attempt::Failed(reason) => FuseInstallOutcome::Failed { reason },
    }
}

/// The package step of FUSE for this system.
fn install_fuse(probe: &RuntimeProbe<'_>) -> Attempt {
    package_install::ensure(Component::Fuse, &|| readiness(probe).is_ok())
}

/// `se drive install-runtime` under Linux.
pub fn install_outcome(probe: &RuntimeProbe<'_>) -> FuseInstallOutcome {
    install_with(probe, || install_fuse(probe))
}

/// The drive host's runtime check: FUSE ready, else installed once and
/// checked once more; the error is the reason the drive shows.
pub fn ensure_ready(probe: &RuntimeProbe<'_>) -> Result<(), String> {
    with_install(|| readiness(probe), || install_fuse(probe))
}

/// The window's note: `None` when FUSE is ready, else the running install,
/// the last refusal, that se installs it at the next mount, or the problem
/// an install cannot change.
pub fn runtime_note(probe: &RuntimeProbe<'_>) -> Option<String> {
    match readiness(probe) {
        Ok(()) => None,
        Err(Failure::Missing(_)) => Some(package_install::missing_note(
            Component::Fuse,
            "beim nächsten Einbinden",
        )),
        Err(Failure::Other(problems)) => Some(format!("FUSE ist nicht bereit: {problems}")),
    }
}

/// The `fusermount3` the host and the unmount use: `FUSERMOUNT_PATH`, the
/// `PATH`, then the standard folders.
pub fn find_helper(probe: &RuntimeProbe<'_>) -> Option<PathBuf> {
    if let Some(explicit) = (probe.env)("FUSERMOUNT_PATH").map(PathBuf::from) {
        if explicit.is_absolute() && below(probe.root, &explicit).is_file() {
            return Some(explicit);
        }
    }
    let path = (probe.env)("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .filter(|directory| directory.is_absolute())
        .chain(STANDARD_DIRS.iter().map(PathBuf::from))
        .map(|directory| directory.join(HELPER))
        .find(|candidate| below(probe.root, candidate).is_file())
}

/// The helper to run, or the bare name for a `PATH` lookup at spawn time.
pub fn helper_program() -> PathBuf {
    find_helper(&RuntimeProbe::system(false)).unwrap_or_else(|| PathBuf::from(HELPER))
}

/// The kernel request timeout in seconds when it is set (> 0).
pub fn kernel_request_timeout() -> Option<u64> {
    sysctl(Path::new("/"), "default_request_timeout").filter(|seconds| *seconds > 0)
}

fn sandbox(probe: &RuntimeProbe<'_>) -> Sandbox {
    if probe.root.join(".flatpak-info").exists() {
        Sandbox::Flatpak
    } else if (probe.env)("SNAP").is_some() {
        Sandbox::Snap
    } else if probe.root.join(".dockerenv").exists()
        || probe.root.join("run/.containerenv").exists()
        || (probe.env)("container").is_some()
    {
        Sandbox::Container
    } else {
        Sandbox::None
    }
}

fn wsl(probe: &RuntimeProbe<'_>) -> bool {
    (probe.env)("WSL_DISTRO_NAME").is_some()
        || (probe.env)("WSL_INTEROP").is_some()
        || probe
            .root
            .join("proc/sys/fs/binfmt_misc/WSLInterop")
            .exists()
        || fs::read_to_string(probe.root.join("proc/version"))
            .is_ok_and(|version| version.to_ascii_lowercase().contains("microsoft"))
}

fn sysctl(root: &Path, name: &str) -> Option<u64> {
    fs::read_to_string(root.join("proc/sys/fs/fuse").join(name))
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn setuid_root(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.uid() == 0 && metadata.permissions().mode() & 0o4000 != 0)
}

fn helper_version(path: &Path) -> Option<String> {
    let output = Command::new(path)
        .arg("-V")
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    text.lines()
        .find_map(|line| line.split_once("version:"))
        .map(|(_, version)| version.trim().to_string())
        .filter(|version| !version.is_empty())
}

fn read_write(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    unsafe { libc::access(path.as_ptr(), libc::R_OK | libc::W_OK) == 0 }
}

/// `path` (absolute) inside the probe root.
fn below(root: &Path, path: &Path) -> PathBuf {
    root.join(path.strip_prefix("/").unwrap_or(path))
}

#[cfg(test)]
#[path = "fuse_runtime_tests.rs"]
mod tests;
