//! `se desktop status` and the `desktop` object of the MCP tool `status`
//! (DSK-M3 contract). Built from reads only: no folder is created, no
//! process and no background service is started. The share state comes from
//! the running shell, or without one from the read without start (V11).
use serde_json::{json, Map, Value};

use super::control::{AutostartState, Integration};
use super::feed_sources::{device_tree_now, DeviceTreeView};
use super::platform;
use super::store::{DesktopDir, ShellRuntime};
use super::Session;
use crate::desktop::core::config::{DesktopConfig, ForeignDesktopIcons};
use crate::desktop::core::ledger::{removal_plan, Registration, Trace};
use crate::desktop::core::mechanism::{Mechanism, SessionFacts};
use crate::desktop::core::state::{Halt, ModuleState, Part, Platform, State};

pub(crate) const DEVICE_TREE_MISSING: &str = "Gerätebaum nicht eingehängt";
pub(crate) const SHELL_NOT_RUNNING: &str = "se Desktop läuft nicht";
pub(crate) const PART_OFF: &str = "Teil ist ausgeschaltet";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PartStatus {
    pub(crate) part: Part,
    pub(crate) enabled: bool,
    pub(crate) supported: bool,
    pub(crate) active: bool,
    pub(crate) reason: Option<String>,
    /// Short code for the text output (`unavailable:<code>`).
    pub(crate) unavailable: Option<String>,
}

/// Everything `se desktop status` reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StatusReport {
    pub(crate) platform: Platform,
    pub(crate) state: State,
    pub(crate) state_error: Option<String>,
    pub(crate) parts: Vec<PartStatus>,
    pub(crate) mechanism: Mechanism,
    pub(crate) shell_pid: Option<u32>,
    pub(crate) surfaces: Vec<(String, u32)>,
    pub(crate) monitors: usize,
    pub(crate) share_live: bool,
    pub(crate) share_reason: Option<String>,
    pub(crate) daemon_autostart: bool,
    pub(crate) autostart: AutostartState,
    pub(crate) halted: Option<Halt>,
    /// `(kind, id, state)` of a foreign desktop program (C2.5, C2.2).
    pub(crate) foreign_desktop: Option<(String, String, String)>,
    pub(crate) registrations: Vec<(Trace, bool)>,
    /// `(kind, target)` of every trace left after `disable`/`remove`.
    pub(crate) residue: Vec<(String, String)>,
    pub(crate) layout_saved: bool,
    pub(crate) config: Option<DesktopConfig>,
}

/// What the shell or the read without start says about Share.
pub(crate) fn share_view(shell: Option<&ShellRuntime>) -> (bool, Option<String>) {
    if let Some(shell) = shell {
        return (
            shell.share_state == "live",
            shell.share_state_reason.clone(),
        );
    }
    match crate::daemon::share_worker_peek() {
        Ok(Some(_)) => (true, None),
        Ok(None) => (false, Some(super::model_feed::NO_SERVICE.into())),
        Err(error) => (false, Some(error)),
    }
}

fn foreign_desktop(
    facts: &SessionFacts,
    config: Option<&DesktopConfig>,
    module: Option<&ModuleState>,
) -> Option<(String, String, String)> {
    let taken = module.and_then(|module| {
        module
            .ledger
            .iter()
            .find(|entry| entry.kind == Registration::ForeignExtensionDisabled)
    });
    if let Some(entry) = taken {
        return Some(("ding".into(), entry.target.clone(), "taken_over".into()));
    }
    if let Some(uuid) = &facts.foreign_desktop_icons {
        let keep =
            config.is_none_or(|config| config.foreign_desktop_icons == ForeignDesktopIcons::Keep);
        let state = if keep { "blocking" } else { "active" };
        return Some(("ding".into(), uuid.clone(), state.into()));
    }
    if facts.cosmic_files_desktop {
        return Some((
            "cosmic_files".into(),
            "cosmic-files-applet".into(),
            "blocking".into(),
        ));
    }
    None
}

/// Collects the report of this user's se Desktop.
pub(crate) fn gather(dir: &DesktopDir, integration: &dyn Integration) -> StatusReport {
    let (module, state_error) = match dir.read_state() {
        Ok(module) => (module, None),
        Err(error) => (None, Some(error)),
    };
    let config = dir.read_config().ok().flatten();
    let session = Session::probe(config.as_ref());
    let running = platform::instance::is_running(dir);
    let shell = running.then(|| dir.read_shell()).flatten().filter(|shell| {
        platform::process::alive_with_image(shell.shell_pid, std::path::Path::new(&shell.exe))
    });
    let (share_live, share_reason) = share_view(shell.as_ref());
    let state = match (&module, &state_error) {
        (Some(module), _) => module.state,
        (None, Some(_)) => State::Disabled,
        (None, None) => State::Absent,
    };
    let parts = part_statuses(
        &session,
        module.as_ref(),
        running,
        shell.as_ref(),
        integration,
    );
    let ledger = module
        .as_ref()
        .map(|module| module.ledger.clone())
        .unwrap_or_default();
    let registrations: Vec<(Trace, bool)> = removal_plan(platform::PLATFORM, &ledger)
        .into_iter()
        .map(|trace| {
            let present = integration.is_applied(&trace);
            (trace, present)
        })
        .collect();
    let mut residue = Vec::new();
    if state != State::Enabled {
        for (trace, present) in &registrations {
            if *present {
                residue.push((trace.kind.key().to_string(), trace.target.clone()));
            }
        }
        for file in integration.runtime_files() {
            residue.push(("runtime_file".into(), file.to_string_lossy().into_owned()));
        }
    }
    StatusReport {
        platform: platform::PLATFORM,
        state,
        state_error,
        parts,
        foreign_desktop: foreign_desktop(&session.facts, config.as_ref(), module.as_ref()),
        mechanism: session.mechanism,
        shell_pid: shell.as_ref().map(|shell| shell.shell_pid),
        surfaces: shell
            .as_ref()
            .map(|shell| {
                shell
                    .surfaces
                    .iter()
                    .map(|surface| (surface.monitor.clone(), surface.pid))
                    .collect()
            })
            .unwrap_or_default(),
        monitors: shell
            .as_ref()
            .map_or_else(|| platform::monitors().len(), |shell| shell.monitors),
        share_live,
        share_reason,
        daemon_autostart: crate::autostart::is_enabled(),
        autostart: integration.autostart(),
        halted: module.as_ref().and_then(|module| module.halted.clone()),
        registrations,
        residue,
        layout_saved: dir.layout_saved(),
        config,
    }
}

fn part_statuses(
    session: &Session,
    module: Option<&ModuleState>,
    running: bool,
    shell: Option<&ShellRuntime>,
    integration: &dyn Integration,
) -> Vec<PartStatus> {
    let enabled_module = module.is_some_and(|module| module.state == State::Enabled);
    Part::ALL
        .into_iter()
        .map(|part| {
            let enabled = module.is_some_and(|module| module.parts.contains(part));
            let missing = session
                .unavailable
                .iter()
                .find(|(missing, _)| *missing == part);
            let code = session
                .unavailable_codes
                .iter()
                .find(|(missing, _)| *missing == part);
            let supported = part.exists_on(platform::PLATFORM);
            let active = match part {
                Part::Surface => shell.is_some_and(|shell| !shell.surfaces.is_empty()),
                Part::NavPane => integration.is_applied(&Trace {
                    kind: Registration::NavPaneClsid,
                    target: crate::desktop::core::ledger::DESKTOP_NAV_CLSID.into(),
                }),
                // DSK-M5 lets the running shell write the list.
                Part::JumpList => enabled && enabled_module && running,
            };
            let reason = match (missing, active, enabled) {
                (Some((_, reason)), _, _) => Some(reason.clone()),
                (None, true, _) => None,
                (None, false, false) => Some(PART_OFF.into()),
                (None, false, true) if part == Part::NavPane => Some(DEVICE_TREE_MISSING.into()),
                (None, false, true) => Some(SHELL_NOT_RUNNING.into()),
            };
            PartStatus {
                part,
                enabled,
                supported,
                active,
                reason,
                unavailable: code.map(|(_, code)| (*code).to_string()),
            }
        })
        .collect()
}

impl StatusReport {
    /// The JSON object of `se desktop status --json` and of MCP `status`.
    pub(crate) fn json(&self) -> Value {
        let mut parts = Map::new();
        for part in &self.parts {
            parts.insert(
                part.part.key().into(),
                json!({
                    "enabled": part.enabled,
                    "supported": part.supported,
                    "active": part.active,
                    "reason": part.reason,
                }),
            );
        }
        let mechanism_reason = match &self.mechanism {
            Mechanism::Unavailable(reason) => Some(reason.text()),
            _ => None,
        };
        let surfaces: Vec<Value> = self
            .surfaces
            .iter()
            .map(|(monitor, pid)| json!({ "monitor": monitor, "pid": pid }))
            .collect();
        let registrations: Vec<Value> = self
            .registrations
            .iter()
            .map(|(trace, present)| {
                json!({ "kind": trace.kind.key(), "target": trace.target, "present": present })
            })
            .collect();
        let residue: Vec<Value> = self
            .residue
            .iter()
            .map(|(kind, target)| json!({ "kind": kind, "target": target }))
            .collect();
        let mut object = Map::new();
        let mut put = |key: &str, value: Value| {
            object.insert(key.into(), value);
        };
        put("platform", json!(platform_key(self.platform)));
        put("state", json!(self.state.key()));
        put("state_error", json!(self.state_error));
        put("parts", Value::Object(parts));
        put("mechanism", json!(self.mechanism.key()));
        put("mechanism_reason", json!(mechanism_reason));
        // GNOME extension states join with DSK-M8.
        put("extension_state", Value::Null);
        put(
            "processes",
            json!({ "shell_pid": self.shell_pid, "surfaces": surfaces }),
        );
        put(
            "share_state",
            json!(if self.share_live { "live" } else { "profiles" }),
        );
        put("share_state_reason", json!(self.share_reason));
        put("daemon_autostart", json!(self.daemon_autostart));
        put("autostart", json!(self.autostart.key()));
        put(
            "halted",
            serde_json::to_value(&self.halted).unwrap_or(Value::Null),
        );
        put(
            "foreign_desktop",
            self.foreign_desktop.as_ref().map_or(
                Value::Null,
                |(kind, id, state)| json!({ "kind": kind, "id": id, "state": state }),
            ),
        );
        put("registrations", Value::Array(registrations));
        put("residue", Value::Array(residue));
        // The S2 device tree (`DevicesStatus`, V4), read from a running
        // worker without starting one.
        let tree = device_tree_now();
        let available = matches!(&tree, DeviceTreeView::Known(status) if status.enabled);
        put(
            "devices_folder",
            json!({
                "available": available,
                "mounted": tree.mounted(),
                "root_path": tree.root_path(),
            }),
        );
        put("layout_saved", json!(self.layout_saved));
        put(
            "config",
            serde_json::to_value(&self.config).unwrap_or(Value::Null),
        );
        Value::Object(object)
    }
}

pub(crate) fn platform_key(platform: Platform) -> &'static str {
    match platform {
        Platform::Windows => "windows",
        Platform::Linux => "linux",
    }
}
