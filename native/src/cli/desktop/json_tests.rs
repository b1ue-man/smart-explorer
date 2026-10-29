//! DSK-M3 signal `desktop_task_cli_json_shapes`: the JSON and text shapes
//! of `se desktop` (status, config, restart) as the contract names them.
use std::collections::BTreeSet;

use serde_json::{json, Value};

use super::output::{config_json, config_text, restart_json, status_text};
use crate::desktop::core::config::{DesktopConfig, SystemItems};
use crate::desktop::core::ledger::{Registration, Trace};
use crate::desktop::core::mechanism::{Mechanism, Reason, SessionFacts};
use crate::desktop::core::state::{Halt, HaltReason, Part, Platform, State};
use crate::desktop::os::control::{AutostartState, RestartOutcome};
use crate::desktop::os::status::{PartStatus, StatusReport};
use crate::desktop::os::{Session, WINDOWS_ONLY};
fn sample_config() -> DesktopConfig {
    DesktopConfig::initial(SystemItems::initial(None, None))
}

fn sample_report() -> StatusReport {
    let part =
        |part, enabled, active, reason: Option<&str>, unavailable: Option<&str>| PartStatus {
            part,
            enabled,
            supported: unavailable != Some("windows_only"),
            active,
            reason: reason.map(str::to_string),
            unavailable: unavailable.map(str::to_string),
        };
    StatusReport {
        platform: Platform::Linux,
        state: State::Disabled,
        state_error: None,
        parts: vec![
            part(
                Part::Surface,
                true,
                false,
                Some("se Desktop läuft nicht"),
                None,
            ),
            part(
                Part::NavPane,
                false,
                false,
                Some(WINDOWS_ONLY),
                Some("windows_only"),
            ),
            part(
                Part::JumpList,
                false,
                false,
                Some(WINDOWS_ONLY),
                Some("windows_only"),
            ),
        ],
        mechanism: Mechanism::Unavailable(Reason::NoGraphicalSession),
        extension_state: None,
        shell_pid: Some(41),
        surfaces: vec![("task-a".into(), 42)],
        monitors: 2,
        share_live: false,
        share_reason: Some("Hintergrunddienst läuft nicht".into()),
        daemon_autostart: false,
        autostart: AutostartState::DisabledBySystem,
        halted: Some(Halt {
            reason: HaltReason::CrashLoop,
            version: "0.5.183".into(),
            at: 7,
        }),
        foreign_desktop: Some((
            "ding".into(),
            "ding@rastersoft.com".into(),
            "blocking".into(),
        )),
        registrations: vec![(
            Trace {
                kind: Registration::AutostartFile,
                target: "smart-explorer-desktop.desktop".into(),
            },
            true,
        )],
        residue: vec![(
            "autostart_file".into(),
            "smart-explorer-desktop.desktop".into(),
        )],
        layout_saved: false,
        config: Some(sample_config()),
    }
}

fn keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("object expected: {value}"))
        .keys()
        .cloned()
        .collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| name.to_string()).collect()
}

#[test]
fn desktop_task_cli_json_shapes() {
    let value = sample_report().json();
    assert_eq!(
        keys(&value),
        set(&[
            "platform",
            "state",
            "state_error",
            "parts",
            "mechanism",
            "mechanism_reason",
            "extension_state",
            "processes",
            "share_state",
            "share_state_reason",
            "daemon_autostart",
            "autostart",
            "halted",
            "foreign_desktop",
            "registrations",
            "residue",
            "devices_folder",
            "layout_saved",
            "config",
        ])
    );
    assert_eq!(
        keys(&value["parts"]),
        set(&["surface", "nav_pane", "jump_list"])
    );
    for part in ["surface", "nav_pane", "jump_list"] {
        assert_eq!(
            keys(&value["parts"][part]),
            set(&["enabled", "supported", "active", "reason"])
        );
    }
    assert_eq!(value["parts"]["nav_pane"]["supported"], json!(false));
    assert_eq!(value["parts"]["nav_pane"]["reason"], json!(WINDOWS_ONLY));
    assert_eq!(value["platform"], json!("linux"));
    assert_eq!(value["state"], json!("disabled"));
    assert_eq!(value["mechanism"], json!("none"));
    assert!(value["mechanism_reason"].is_string());
    assert_eq!(value["extension_state"], Value::Null);
    assert_eq!(keys(&value["processes"]), set(&["shell_pid", "surfaces"]));
    assert_eq!(
        value["processes"]["surfaces"],
        json!([{"monitor": "task-a", "pid": 42}])
    );
    assert_eq!(value["share_state"], json!("profiles"));
    assert_eq!(value["autostart"], json!("disabled_by_system"));
    assert_eq!(
        value["halted"],
        json!({"reason": "crash_loop", "version": "0.5.183", "at": 7})
    );
    assert_eq!(
        value["foreign_desktop"],
        json!({"kind": "ding", "id": "ding@rastersoft.com", "state": "blocking"})
    );
    assert_eq!(
        keys(&value["registrations"][0]),
        set(&["kind", "target", "present"])
    );
    assert_eq!(keys(&value["residue"][0]), set(&["kind", "target"]));
    assert_eq!(
        value["devices_folder"],
        json!({"available": false, "mounted": false, "root_path": null})
    );
    assert_eq!(value["layout_saved"], json!(false));
    assert_eq!(
        keys(&value["config"]),
        set(&[
            "schema",
            "background",
            "groups",
            "system_items",
            "open_devices_with",
            // Added with folder tiles (1aea1216, `DesktopConfig::open_folders_with`).
            "open_folders_with",
            "foreign_desktop_icons",
        ])
    );

    // Text: one line per contract field, tab-separated.
    let text = status_text(&sample_report());
    let firsts: Vec<&str> = text
        .lines()
        .map(|line| line.split('\t').next().unwrap())
        .collect();
    assert_eq!(
        firsts,
        [
            "state",
            "part",
            "part",
            "part",
            "mechanism",
            "surfaces",
            "share_state",
            "autostart",
            "halted",
            "foreign_desktop",
            "residue",
        ]
    );
    assert!(text.contains("part\tnav_pane\toff\tunavailable:windows_only\n"));
    assert!(text.contains("mechanism\tnone:no_graphical_session\n"));
    assert!(text.contains("surfaces\t1/2\n"));
    assert!(text.contains("halted\tcrash_loop:0.5.183\n"));
    assert!(text.contains("foreign_desktop\tding:ding@rastersoft.com:blocking\n"));
    assert!(text.contains("residue\t1\n"));

    // Linux parts outside Windows carry the contract reason.
    let session = Session::from_facts(Platform::Linux, SessionFacts::default(), None);
    assert!(session
        .unavailable
        .contains(&(Part::NavPane, WINDOWS_ONLY.into())));
    assert!(session
        .unavailable
        .contains(&(Part::JumpList, WINDOWS_ONLY.into())));

    let restart = RestartOutcome {
        restarted: false,
        state: State::Absent,
        reason: Some("se Desktop ist nicht hinzugefügt".into()),
        failed: false,
    };
    assert_eq!(
        keys(&restart_json(&restart)),
        set(&["restarted", "state", "reason"])
    );
    assert_eq!(restart_json(&restart)["state"], json!("absent"));
    assert_eq!(keys(&config_json(Some(&sample_config()))), set(&["config"]));
    assert_eq!(config_json(None), json!({"config": null}));
    assert!(config_text(Some(&sample_config())).starts_with("background\tsystem\n"));
}
