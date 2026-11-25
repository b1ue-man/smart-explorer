//! Admits se's fixed device ports (`firewall_linux_plan::DEVICE_PORTS`) in the
//! active host firewall (user orders 2026-11-25: „update, installation,
//! NUTZUNG richten das ein“, „ELEVATION FEHLT? ERFRAGEN!“). At use, without
//! root, the firewall services every account can see and the root step's
//! record decide; nothing is asked while no firewall runs or the record
//! answers for the running ones. Otherwise the root step `se rdp host-setup
//! --system firewall-peer` runs: in this process as root, else through
//! pkexec (the polkit dialog of se's install-on-use, `package_install.rs`),
//! one dialog at a time for this account (`install_gate`). A refused dialog
//! is reported with the missing ports and asked again at the next use.
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::{Command, Output, Stdio};

use super::firewall_linux_plan::{self as plan, Backend, Record, StepOutcome, DEVICE_PORTS};
use crate::login::os::shared::install_gate::{Attempt, Gate};
use crate::login::os::shared::self_install::Elevation;

/// The root step's name (`se rdp host-setup --system firewall-peer`).
pub const STEP: &str = "firewall-peer";
const GATE: &str = "firewall-peer";
/// Exit code of a failed root step (package install 60–62, display 50–55).
const FAILED: i32 = 70;
/// The record of ufw, firewalld or no filtering firewall (kept on disk).
const RECORD_KEPT: &str = "/var/lib/smart-explorer/firewall-peer";
/// The record of rules added to an nftables or iptables ruleset: they end
/// with the boot, and so does this file.
const RECORD_BOOT: &str = "/run/smart-explorer-firewall-peer";
const RECORD_LIMIT: u64 = 4096;

const UFW: [&str; 2] = ["/usr/sbin/ufw", "/sbin/ufw"];
const FIREWALL_CMD: [&str; 2] = ["/usr/bin/firewall-cmd", "/bin/firewall-cmd"];
const NFT: [&str; 2] = ["/usr/sbin/nft", "/sbin/nft"];
const IPTABLES: [[&str; 2]; 2] = [
    ["/usr/sbin/iptables", "/sbin/iptables"],
    ["/usr/sbin/ip6tables", "/sbin/ip6tables"],
];
const SYSTEMCTL: [&str; 2] = ["/usr/bin/systemctl", "/bin/systemctl"];

fn program(candidates: &[&'static str]) -> Option<&'static str> {
    candidates
        .iter()
        .copied()
        .find(|path| Path::new(path).is_file())
}

fn run(program: &str, args: &[String]) -> io::Result<Output> {
    Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|arg| arg.to_string()).collect()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The program's own last message line for an error text.
fn message(program: &str, output: &Output) -> String {
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let line = text
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("ohne Meldung");
    format!("{program}: {}", line.chars().take(300).collect::<String>())
}

fn must(program: &str, call: &[String]) -> Result<(), String> {
    let output = run(program, call).map_err(|error| format!("{program}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(message(program, &output))
    }
}

fn service_active(name: &str) -> bool {
    program(&SYSTEMCTL).is_some_and(|systemctl| {
        Command::new(systemctl)
            .args(["is-active", "--quiet", name])
            .stdin(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

/// The firewalls any account sees running: ufw enabled in its readable
/// configuration, and the services of firewalld, nftables and iptables.
fn visible_firewalls() -> Vec<Backend> {
    let mut found = Vec::new();
    let ufw =
        std::fs::read_to_string("/etc/ufw/ufw.conf").is_ok_and(|conf| plan::ufw_enabled(&conf));
    if ufw && program(&UFW).is_some() {
        found.push(Backend::Ufw);
    }
    if service_active("firewalld") {
        found.push(Backend::Firewalld);
    }
    if service_active("nftables") {
        found.push(Backend::Nftables);
    }
    if ["iptables", "ip6tables", "netfilter-persistent"]
        .into_iter()
        .any(service_active)
    {
        found.push(Backend::Iptables);
    }
    found
}

/// A file only root can have written (owner root, no group or other write,
/// in a folder of the same kind, no link).
fn root_written(path: &Path) -> bool {
    let trusted = |path: &Path| {
        std::fs::symlink_metadata(path).is_ok_and(|meta| {
            meta.uid() == 0 && meta.mode() & 0o022 == 0 && !meta.file_type().is_symlink()
        })
    };
    trusted(path) && path.parent().is_some_and(trusted)
}

fn record_at(path: &str) -> Option<Record> {
    let path = Path::new(path);
    if !root_written(path) || std::fs::metadata(path).ok()?.len() > RECORD_LIMIT {
        return None;
    }
    Record::parse(&std::fs::read_to_string(path).ok()?)
}

/// The record that answers for the firewalls running now.
fn fitting_record(visible: &[Backend]) -> Option<Record> {
    [RECORD_BOOT, RECORD_KEPT]
        .into_iter()
        .filter_map(record_at)
        .find(|record| record.fits(DEVICE_PORTS, visible))
}

fn labels() -> String {
    let labels: Vec<String> = DEVICE_PORTS.iter().map(|port| port.label()).collect();
    labels.join(", ")
}

/// Without asking (the update helper): the state, or what the next start
/// of se asks for.
pub fn state_without_prompt() -> String {
    let visible = visible_firewalls();
    if visible.is_empty() {
        return plan::status_line(None, DEVICE_PORTS);
    }
    match fitting_record(&visible) {
        Some(record) => plan::status_line(record.backend, DEVICE_PORTS),
        None => {
            let names: Vec<&str> = visible.iter().map(|backend| backend.name()).collect();
            format!(
                "Firewall ({}): {} noch nicht freigegeben; se fragt beim nächsten Start nach der Freigabe",
                names.join(", "),
                labels()
            )
        }
    }
}

/// At use (the Share worker's start, before peers connect).
pub fn ensure_at_use() -> io::Result<String> {
    let visible = visible_firewalls();
    if visible.is_empty() {
        return Ok(plan::status_line(None, DEVICE_PORTS));
    }
    if let Some(record) = fitting_record(&visible) {
        return Ok(plan::status_line(record.backend, DEVICE_PORTS));
    }
    // A test build never changes the firewall of the machine it runs on.
    if cfg!(any(test, feature = "test-support")) {
        return Err(io::Error::other(format!(
            "Testbau ändert keine Firewall; nicht freigegeben: {}",
            labels()
        )));
    }
    let present = || fitting_record(&visible_firewalls()).is_some();
    match Gate::named(GATE).run(&present, elevate) {
        Attempt::Installed => fitting_record(&visible_firewalls())
            .map(|record| plan::status_line(record.backend, DEVICE_PORTS))
            .ok_or_else(|| {
                io::Error::other(format!(
                    "der Firewall-Schritt meldete Erfolg, sein Eintrag fehlt aber; nicht freigegeben: {}",
                    labels()
                ))
            }),
        Attempt::Declined(reason) => Err(io::Error::new(io::ErrorKind::PermissionDenied, reason)),
        Attempt::Failed(reason) => Err(io::Error::other(reason)),
    }
}

fn attempt(outcome: StepOutcome) -> Attempt {
    match outcome {
        StepOutcome::Done => Attempt::Installed,
        StepOutcome::Declined(reason) => Attempt::Declined(reason),
        StepOutcome::Failed(reason) => Attempt::Failed(reason),
    }
}

/// Root runs the step itself; any other account through pkexec.
fn elevate() -> Attempt {
    let missing = labels();
    match crate::login::os::platform::elevation() {
        Ok(Elevation::InProcess) => {
            let (line, code) = run_step();
            attempt(plan::step_outcome(Some(code), &line, "", &missing))
        }
        Ok(Elevation::External(se)) => {
            crate::notify_desktop::notify_access(
                "Smart Explorer",
                &format!("se gibt {missing} in der Firewall frei; das System fragt nach der Freigabe."),
            );
            match crate::login::os::platform::run_elevated_launched(&se, &[STEP.to_string()]) {
                Ok(launched) => attempt(plan::step_outcome(
                    launched.code,
                    &launched.stdout,
                    &launched.stderr,
                    &missing,
                )),
                Err(error) => Attempt::Failed(format!("{error}; nicht freigegeben: {missing}")),
            }
        }
        Err(reason) if reason == "no_prompt" => Attempt::Failed(format!(
            "pkexec fehlt, das System kann nicht nach der Freigabe fragen; nicht freigegeben: {missing}"
        )),
        Err(reason) => Attempt::Failed(format!("{reason}; nicht freigegeben: {missing}")),
    }
}

/// `se rdp host-setup --system firewall-peer` (root): the line and the exit
/// code, or `None` for another step.
pub fn system_step(step: &str) -> Option<(String, i32)> {
    (step == STEP).then(run_step)
}

fn run_step() -> (String, i32) {
    match admit_as_root() {
        Ok(backend) => (
            format!(
                "firewall_state=done:{}",
                backend.map_or("none", Backend::name)
            ),
            0,
        ),
        Err(message) => (format!("firewall_state=failed:{message}"), FAILED),
    }
}

/// The filtering firewall as root sees it, in the order its rules decide:
/// ufw and firewalld manage their own nftables/iptables rules.
fn detect_as_root() -> Option<Backend> {
    let ok = |output: io::Result<Output>| output.ok().filter(|output| output.status.success());
    if let Some(ufw) = program(&UFW) {
        if ok(run(ufw, &args(&["status"]))).is_some_and(|output| plan::ufw_active(&stdout(&output)))
        {
            return Some(Backend::Ufw);
        }
    }
    if let Some(firewall_cmd) = program(&FIREWALL_CMD) {
        if ok(run(firewall_cmd, &args(&["--state"]))).is_some() {
            return Some(Backend::Firewalld);
        }
    }
    if let Some(nft) = program(&NFT) {
        let ruleset = ok(run(nft, &args(&["list", "ruleset"]))).map(|output| stdout(&output));
        if ruleset.is_some_and(|text| {
            plan::nft_input_chains(&text)
                .iter()
                .any(plan::NftChain::filters)
        }) {
            return Some(Backend::Nftables);
        }
    }
    legacy_iptables()
        .iter()
        .any(|binary| filters_iptables(binary))
        .then_some(Backend::Iptables)
}

/// The legacy iptables programs (nftables shows the rules of the nf_tables
/// variant itself).
fn legacy_iptables() -> Vec<&'static str> {
    IPTABLES
        .iter()
        .filter_map(|candidates| program(candidates))
        .filter(|binary| {
            run(binary, &args(&["--version"]))
                .is_ok_and(|output| plan::iptables_legacy(&stdout(&output)))
        })
        .collect()
}

fn filters_iptables(binary: &str) -> bool {
    run(binary, &args(&["-S", "INPUT"]))
        .is_ok_and(|output| plan::iptables_filters(&stdout(&output)))
}

fn admit_as_root() -> Result<Option<Backend>, String> {
    let backend = detect_as_root();
    match backend {
        None => {}
        Some(Backend::Ufw) => {
            let ufw = program(&UFW).ok_or("ufw fehlt")?;
            for port in DEVICE_PORTS {
                must(ufw, &plan::ufw_args(*port))?;
            }
        }
        Some(Backend::Firewalld) => {
            let firewall_cmd = program(&FIREWALL_CMD).ok_or("firewall-cmd fehlt")?;
            let query = |call: &[&str]| {
                run(firewall_cmd, &args(call))
                    .map(|output| stdout(&output))
                    .unwrap_or_default()
            };
            let zones = plan::firewalld_zones(
                &query(&["--get-active-zones"]),
                &query(&["--get-default-zone"]),
            );
            for zone in zones {
                for call in plan::firewalld_args(&zone, DEVICE_PORTS) {
                    must(firewall_cmd, &call)?;
                }
            }
        }
        Some(Backend::Nftables) => {
            let nft = program(&NFT).ok_or("nft fehlt")?;
            let ruleset = run(nft, &args(&["list", "ruleset"]))
                .map(|output| stdout(&output))
                .map_err(|error| format!("nft: {error}"))?;
            for chain in plan::nft_input_chains(&ruleset)
                .iter()
                .filter(|chain| chain.filters())
            {
                for port in DEVICE_PORTS.iter().filter(|port| !chain.admits(**port)) {
                    must(nft, &chain.insert_args(*port))?;
                }
            }
        }
        Some(Backend::Iptables) => {
            for binary in legacy_iptables()
                .into_iter()
                .filter(|binary| filters_iptables(binary))
            {
                for port in DEVICE_PORTS {
                    let present = run(binary, &plan::iptables_args(true, *port))
                        .is_ok_and(|output| output.status.success());
                    if !present {
                        must(binary, &plan::iptables_args(false, *port))?;
                    }
                }
            }
        }
    }
    write_record(backend)?;
    Ok(backend)
}

/// The record next to the rules' own lifetime; the other one goes.
fn write_record(backend: Option<Backend>) -> Result<(), String> {
    let (path, other) = if backend.is_some_and(|backend| !backend.persistent()) {
        (RECORD_BOOT, RECORD_KEPT)
    } else {
        (RECORD_KEPT, RECORD_BOOT)
    };
    let text = plan::record_text(backend, DEVICE_PORTS, &visible_firewalls());
    let path = Path::new(path);
    let folder = path.parent().ok_or("Ordner des Firewall-Eintrags fehlt")?;
    std::fs::create_dir_all(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    let temporary = path.with_extension("new");
    std::fs::write(&temporary, text)
        .and_then(|()| std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o644)))
        .and_then(|()| std::fs::rename(&temporary, path))
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let _ = std::fs::remove_file(other);
    Ok(())
}
