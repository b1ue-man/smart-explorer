//! Install-on-use of the Linux packages se needs (user order 2026-11-24):
//! FUSE for drives, a FreeRDP 3 client for Remote Desktop and dnsmasq for
//! the LAN uplink; the virtual display's package step uses `install_as_root`.
//! The caller asks once through the install gate (`install_gate.rs`): root
//! installs itself, any other account runs `pkexec <se> rdp host-setup
//! --system package-install:<id>` (the polkit dialog is the only
//! interaction). The elevated `se` picks the package from the fixed table
//! below; no package name crosses the elevation boundary.
use std::path::Path;
use std::process::{Command, Output, Stdio};

use crate::login::os::shared::install_gate::{Attempt, Gate, Progress};
use crate::login::os::shared::self_install::Elevation;

/// The step name prefix of `se rdp host-setup --system`.
pub const STEP_PREFIX: &str = "package-install:";

/// Exit codes of the package step (login uses 40–47, the display 50–55).
pub mod exit {
    pub const PACKAGE: i32 = 60;
    pub const DISTRIBUTION: i32 = 61;
    pub const STEP: i32 = 62;
}

const OS_RELEASE: [&str; 2] = ["/etc/os-release", "/usr/lib/os-release"];
const MESSAGE_LIMIT: usize = 400;

/// A component se installs on first use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component {
    Fuse,
    FreeRdp,
    Dnsmasq,
}

impl Component {
    pub const ALL: [Component; 3] = [Component::Fuse, Component::FreeRdp, Component::Dnsmasq];

    pub fn id(self) -> &'static str {
        match self {
            Self::Fuse => "fuse",
            Self::FreeRdp => "freerdp",
            Self::Dnsmasq => "dnsmasq",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|component| component.id() == id)
    }

    /// The name the messages use.
    pub fn label(self) -> &'static str {
        match self {
            Self::Fuse => "FUSE",
            Self::FreeRdp => "FreeRDP 3",
            Self::Dnsmasq => "dnsmasq",
        }
    }

    fn gate(self) -> Gate {
        Gate::named(&format!("package-{}", self.id()))
    }

    /// The elevated step of this component.
    pub fn step(self) -> String {
        format!("{STEP_PREFIX}{}", self.id())
    }
}

/// The package managers se drives (unattended, from the distribution's
/// signed repositories).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manager {
    AptGet,
    Dnf,
    Pacman,
    Zypper,
}

impl Manager {
    const PATHS: [(Manager, &'static str); 4] = [
        (Manager::AptGet, "/usr/bin/apt-get"),
        (Manager::Dnf, "/usr/bin/dnf"),
        (Manager::Pacman, "/usr/bin/pacman"),
        (Manager::Zypper, "/usr/bin/zypper"),
    ];

    /// The first package manager present on this system.
    pub fn detect() -> Option<Manager> {
        Self::PATHS
            .into_iter()
            .find(|(_, path)| Path::new(path).is_file())
            .map(|(manager, _)| manager)
    }

    fn program(self) -> &'static str {
        Self::PATHS
            .into_iter()
            .find(|(manager, _)| *manager == self)
            .map_or("", |(_, path)| path)
    }

    /// The arguments that install `package` without questions.
    pub fn install_args(self, package: &str) -> Vec<String> {
        let args: &[&str] = match self {
            // Waits for an unattended upgrade that holds the lock (apt 1.9.11+).
            Self::AptGet => &[
                "-o",
                "DPkg::Lock::Timeout=300",
                "install",
                "-y",
                "--no-install-recommends",
            ],
            Self::Dnf => &["install", "-y"],
            Self::Pacman => &["-S", "--noconfirm", "--needed"],
            Self::Zypper => &["--non-interactive", "install"],
        };
        args.iter()
            .map(|arg| arg.to_string())
            .chain([package.to_string()])
            .collect()
    }

    /// The family whose package names this manager's repositories use.
    pub fn family(self) -> Family {
        match self {
            Self::AptGet => Family::Debian,
            Self::Dnf => Family::Fedora,
            Self::Pacman => Family::Arch,
            Self::Zypper => Family::Suse,
        }
    }
}

/// The distribution family that decides the package names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Debian,
    Fedora,
    Arch,
    Suse,
}

/// The family of the `ID`/`ID_LIKE` values of os-release; the package
/// manager decides when they name none se knows.
pub fn family(ids: &[String], manager: Option<Manager>) -> Option<Family> {
    let has = |wanted: &[&str]| ids.iter().any(|id| wanted.contains(&id.as_str()));
    if has(&["debian", "ubuntu"]) {
        Some(Family::Debian)
    } else if has(&["fedora", "rhel", "centos"]) {
        Some(Family::Fedora)
    } else if has(&["arch", "manjaro"]) {
        Some(Family::Arch)
    } else if has(&["suse", "sles"]) || ids.iter().any(|id| id.starts_with("opensuse")) {
        Some(Family::Suse)
    } else {
        manager.map(Manager::family)
    }
}

/// The packages of `component` for `family`, in the order the step tries
/// them: the first one the repositories offer is installed. FreeRDP's SDL
/// client asks for the account in its own dialog; Debian and Ubuntu
/// releases without the package `freerdp3-sdl` (Ubuntu 24.04) get the X11
/// client. Debian's `dnsmasq-base` is the program NetworkManager runs,
/// without the system DNS service of the package `dnsmasq`.
pub fn packages(component: Component, family: Family) -> &'static [&'static str] {
    match (component, family) {
        (Component::Fuse, _) => &["fuse3"],
        (Component::FreeRdp, Family::Debian) => &["freerdp3-sdl", "freerdp3-x11"],
        (Component::FreeRdp, _) => &["freerdp"],
        (Component::Dnsmasq, Family::Debian) => &["dnsmasq-base"],
        (Component::Dnsmasq, _) => &["dnsmasq"],
    }
}

fn system_ids() -> Vec<String> {
    let text = OS_RELEASE
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    crate::screen::core::virtual_display_config::os_release_ids(&text)
}

/// The packages se would install for `component` on this system.
pub fn planned_packages(component: Component) -> Option<&'static [&'static str]> {
    let manager = Manager::detect();
    family(&system_ids(), manager).map(|family| packages(component, family))
}

/// „Paket fuse3“ or „Paket freerdp3-sdl oder freerdp3-x11“.
pub fn package_text(component: Component) -> String {
    match planned_packages(component) {
        Some(packages) => format!("Paket {}", packages.join(" oder ")),
        None => format!("Paket für {}", component.label()),
    }
}

/// A failed package step as root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootError {
    /// None of apt-get, dnf, pacman or zypper is present.
    NoManager,
    /// The package manager's own message.
    Package(String),
}

/// The last non-empty line of the error output, else of the output.
pub fn message_of(stdout: &str, stderr: &str) -> String {
    let last = |text: &str| {
        text.lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_string)
    };
    let line = last(stderr).or_else(|| last(stdout)).unwrap_or_default();
    line.chars()
        .filter(|c| !c.is_control())
        .take(MESSAGE_LIMIT)
        .collect()
}

fn run_logged(command: &mut Command) -> Result<Output, String> {
    command
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| error.to_string())
}

fn output_message(output: &Output) -> String {
    message_of(
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    )
}

fn install_one(manager: Manager, package: &str) -> Result<(), String> {
    let mut command = Command::new(manager.program());
    command
        .env("DEBIAN_FRONTEND", "noninteractive")
        .args(manager.install_args(package));
    let output = run_logged(&mut command)?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!("{package}: {}", output_message(&output)))
}

fn first_installed(manager: Manager, packages: &[&str]) -> Result<(), String> {
    let mut last = String::new();
    for package in packages {
        match install_one(manager, package) {
            Ok(()) => return Ok(()),
            Err(message) => last = message,
        }
    }
    Err(last)
}

/// Installs the first of `packages` the repositories offer (as root). Stale
/// package lists of Debian and Ubuntu are refreshed once, then it tries
/// again. The error carries the package manager's own last message.
pub fn install_as_root(packages: &[&str]) -> Result<(), RootError> {
    let manager = Manager::detect().ok_or(RootError::NoManager)?;
    let first = match first_installed(manager, packages) {
        Ok(()) => return Ok(()),
        Err(message) => message,
    };
    if manager != Manager::AptGet {
        return Err(RootError::Package(first));
    }
    let mut update = Command::new(manager.program());
    update.env("DEBIAN_FRONTEND", "noninteractive").args([
        "-o",
        "DPkg::Lock::Timeout=300",
        "update",
    ]);
    match run_logged(&mut update) {
        Ok(output) if output.status.success() => {
            first_installed(manager, packages).map_err(RootError::Package)
        }
        _ => Err(RootError::Package(first)),
    }
}

/// The root part of `component`: its package, and for FUSE the kernel module
/// when `/dev/fuse` is absent.
fn install_component_as_root(component: Component) -> Result<(), RootError> {
    let manager = Manager::detect().ok_or(RootError::NoManager)?;
    let family = family(&system_ids(), Some(manager)).ok_or(RootError::NoManager)?;
    install_as_root(packages(component, family))?;
    if component == Component::Fuse && !Path::new("/dev/fuse").exists() {
        let modprobe = ["/usr/sbin/modprobe", "/sbin/modprobe"]
            .into_iter()
            .find(|path| Path::new(path).is_file())
            .ok_or_else(|| RootError::Package("modprobe fehlt".into()))?;
        let output = run_logged(Command::new(modprobe).arg("fuse")).map_err(RootError::Package)?;
        if !output.status.success() {
            let message = output_message(&output);
            return Err(RootError::Package(format!("modprobe fuse: {message}")));
        }
    }
    Ok(())
}

/// `se rdp host-setup --system package-install:<id>` (root): the line and
/// the exit code, or `None` for another step.
pub fn system_step(step: &str) -> Option<(String, i32)> {
    let id = step.strip_prefix(STEP_PREFIX)?;
    let Some(component) = Component::from_id(id) else {
        return Some(("package_state=failed:unknown_step".into(), exit::STEP));
    };
    Some(match install_component_as_root(component) {
        Ok(()) => ("package_state=done".into(), 0),
        Err(RootError::NoManager) => (
            "package_state=failed:distribution_unknown".into(),
            exit::DISTRIBUTION,
        ),
        Err(RootError::Package(message)) => (
            format!("package_state=failed:package:{message}"),
            exit::PACKAGE,
        ),
    })
}

fn no_manager_text(component: Component) -> String {
    format!(
        "{} kann nicht installiert werden: auf diesem System gibt es weder apt-get, dnf, pacman noch zypper.",
        component.label()
    )
}

fn declined_text(component: Component, packages: &str) -> String {
    format!(
        "Die Installation von {} ({packages}) wurde in der Systemabfrage abgelehnt.",
        component.label()
    )
}

fn root_attempt(component: Component, packages: &str, result: Result<(), RootError>) -> Attempt {
    match result {
        Ok(()) => Attempt::Installed,
        Err(RootError::NoManager) => Attempt::Failed(no_manager_text(component)),
        Err(RootError::Package(message)) => Attempt::Failed(format!(
            "Die Paketverwaltung konnte {} ({packages}) nicht installieren: {message}",
            component.label()
        )),
    }
}

/// What the pkexec call of the package step reported: its exit code, the
/// child's `package_state=` line and pkexec's own error output (126: the
/// dialog was dismissed, 127: not authorized or no polkit agent).
pub fn attempt_from(
    component: Component,
    packages: &str,
    code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> Attempt {
    let state = stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("package_state="));
    match (code, state) {
        (Some(0), _) | (_, Some("done")) => Attempt::Installed,
        (_, Some("failed:distribution_unknown")) => Attempt::Failed(no_manager_text(component)),
        (_, Some(state)) if state.starts_with("failed:package:") => {
            let message = state.trim_start_matches("failed:package:").to_string();
            root_attempt(component, packages, Err(RootError::Package(message)))
        }
        (Some(126), None) => Attempt::Declined(declined_text(component, packages)),
        (Some(127), None) if stderr.contains("No authentication agent") => {
            Attempt::Failed(format!(
                "{} ({packages}) kann nicht installiert werden: in dieser Sitzung läuft kein Polkit-Agent, der nach der Freigabe fragt.",
                component.label()
            ))
        }
        (Some(127), None) => Attempt::Declined(declined_text(component, packages)),
        (code, _) => Attempt::Failed(format!(
            "Die Installation von {} ({packages}) endete mit {}: {}",
            component.label(),
            code.map_or("einem Signal".to_string(), |code| format!("Code {code}")),
            message_of(stdout, stderr)
        )),
    }
}

/// Root runs the package step itself; any other account through pkexec.
fn install_elevated(component: Component) -> Attempt {
    let packages = package_text(component);
    if cfg!(any(test, feature = "test-support")) {
        return Attempt::Failed(format!(
            "{} ({packages}) wird in einem Testbau nicht installiert.",
            component.label()
        ));
    }
    match crate::login::os::platform::elevation() {
        Ok(Elevation::InProcess) => root_attempt(
            component,
            &packages,
            install_component_as_root(component),
        ),
        Ok(Elevation::External(se)) => {
            crate::notify_desktop::notify_access(
                "Smart Explorer",
                &format!(
                    "{} wird installiert ({packages}); das System fragt nach der Freigabe.",
                    component.label()
                ),
            );
            match crate::login::os::platform::run_elevated_launched(&se, &[component.step()]) {
                Ok(launched) => attempt_from(
                    component,
                    &packages,
                    launched.code,
                    &launched.stdout,
                    &launched.stderr,
                ),
                Err(error) => Attempt::Failed(format!(
                    "{} ({packages}) kann nicht installiert werden: {error}",
                    component.label()
                )),
            }
        }
        Err(reason) if reason == "no_prompt" => Attempt::Failed(format!(
            "{} ({packages}) kann nicht installiert werden: pkexec fehlt, das System kann nicht nach der Freigabe fragen.",
            component.label()
        )),
        Err(reason) => Attempt::Failed(format!(
            "{} ({packages}) kann nicht installiert werden: {reason}",
            component.label()
        )),
    }
}

/// Installs `component` unless `present()`: one install at a time for this
/// account; a caller that waited for a running install takes its result.
pub fn ensure(component: Component, present: &dyn Fn() -> bool) -> Attempt {
    component
        .gate()
        .run(present, || install_elevated(component))
}

/// `ensure` for an automatic caller (the uplink start): after a declined or
/// failed install it never asks again on its own; the person's next action
/// (switching the feature on, „Einrichtung reparieren“) uses `ensure`.
pub fn ensure_unless_refused(component: Component, present: &dyn Fn() -> bool) -> Attempt {
    if present() {
        return Attempt::Installed;
    }
    match component.gate().refusal() {
        Some(refusal) => refusal,
        None => ensure(component, present),
    }
}

/// The reason of the last declined or failed install of `component`.
pub fn refusal(component: Component) -> Option<String> {
    component
        .gate()
        .refusal()
        .and_then(|attempt| attempt.result().err())
}

/// Whether an install of `component` runs now (any process of this account).
pub fn installing(component: Component) -> bool {
    component.gate().progress() == Progress::Running
}

/// While an install runs: „FUSE wird installiert (Paket fuse3) …“.
pub fn progress_text(component: Component) -> Option<String> {
    installing(component).then(|| {
        format!(
            "{} wird installiert ({}) …",
            component.label(),
            package_text(component)
        )
    })
}

/// The desktop's note for a missing `component`: the running install, the
/// last refusal, or that se installs it at the next use (`use_text`).
pub fn missing_note(component: Component, use_text: &str) -> String {
    progress_text(component)
        .or_else(|| refusal(component))
        .unwrap_or_else(|| {
            format!(
                "{} fehlt ({}); se installiert es {use_text} – das System fragt dann nach der Freigabe.",
                component.label(),
                package_text(component)
            )
        })
}

#[cfg(test)]
#[path = "package_install_tests.rs"]
mod tests;
