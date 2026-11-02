//! DSK-M1: module state (C5.3), `state.json` schema 1 (C5.4) and the removal
//! plan of the registration ledger (C5.7).
use super::ledger::{
    removal_plan, LedgerEntry, Registration, Trace, AUTOSTART_FILE_NAME, DESKTOP_NAV_CLSID,
    GNOME_EXTENSION_UUID, JUMP_LIST_TARGET, KNOWN_TRACES, RUN_VALUE_NAME,
};
use super::state::{
    disable, enable, set_part, Action, ModuleState, Part, PartSet, Platform, State, StateError,
};
use serde_json::{json, Value};

const WINDOWS_PARTS: PartSet = PartSet {
    surface: true,
    nav_pane: true,
    jump_list: true,
};
const LINUX_PARTS: PartSet = PartSet {
    surface: true,
    nav_pane: false,
    jump_list: false,
};

fn trace(kind: Registration, target: &str) -> Trace {
    Trace {
        kind,
        target: target.into(),
    }
}

fn entry(kind: Registration, target: &str) -> LedgerEntry {
    LedgerEntry {
        kind,
        target: target.into(),
        applied_at: 1_792_000_001,
    }
}

#[test]
fn desktop_task_state_transitions() {
    use Action::{Disable, Enable, Remove};
    use State::{Absent, Disabled, Enabled};
    let table = [
        (Absent, Enable, Enabled),
        (Absent, Disable, Absent),
        (Absent, Remove, Absent),
        (Enabled, Enable, Enabled),
        (Enabled, Disable, Disabled),
        (Enabled, Remove, Absent),
        (Disabled, Enable, Enabled),
        (Disabled, Disable, Disabled),
        (Disabled, Remove, Absent),
    ];
    for (from, action, to) in table {
        assert_eq!(from.after(action), to, "{from:?} {action:?}");
        assert_eq!(
            from.after(action).after(action),
            to,
            "twice {from:?} {action:?}"
        );
    }
    assert_eq!(State::of(None), State::Absent);

    // absent → enabled: every available part, nothing applied yet.
    let enabled = enable(None, None, WINDOWS_PARTS, 100).unwrap();
    assert_eq!(State::of(Some(&enabled)), State::Enabled);
    assert_eq!(enabled.parts, WINDOWS_PARTS);
    assert_eq!(enabled.changed_at, 100);
    assert!(enabled.ledger.is_empty() && enabled.halted.is_none());
    // A second enable changes nothing, not even the time.
    assert_eq!(
        enable(Some(&enabled), None, WINDOWS_PARTS, 200),
        Ok(enabled.clone())
    );
    // Requested parts only; a requested part that is not available fails.
    let surface_only = enable(None, Some(&[Part::Surface]), WINDOWS_PARTS, 100).unwrap();
    assert_eq!(surface_only.parts, PartSet::of(&[Part::Surface]));
    let linux = enable(None, None, LINUX_PARTS, 100).unwrap();
    assert_eq!(linux.parts, LINUX_PARTS);
    assert_eq!(
        enable(None, Some(&[Part::NavPane]), LINUX_PARTS, 100),
        Err(StateError::PartUnavailable(Part::NavPane))
    );
    assert!(Part::Surface.exists_on(Platform::Linux));
    assert!(!Part::NavPane.exists_on(Platform::Linux));
    assert!(!Part::JumpList.exists_on(Platform::Linux));
    assert!(Part::ALL
        .into_iter()
        .all(|part| part.exists_on(Platform::Windows)));

    // enabled → disabled keeps parts and ledger; a second disable is a no-op.
    let mut applied = enabled.clone();
    applied.record(entry(Registration::RunValue, RUN_VALUE_NAME));
    let disabled = disable(Some(&applied), 300).unwrap();
    assert_eq!(disabled.state, State::Disabled);
    assert_eq!(disabled.parts, applied.parts);
    assert_eq!(disabled.ledger, applied.ledger);
    assert_eq!(disabled.changed_at, 300);
    assert_eq!(disable(Some(&disabled), 400), Some(disabled.clone()));
    assert_eq!(disable(None, 400), None);

    // disabled → enabled restores the stored parts and clears a halt.
    let mut halted = disabled.clone();
    halted.halt("0.5.183", 350);
    let again = enable(Some(&halted), None, WINDOWS_PARTS, 500).unwrap();
    assert_eq!(again.state, State::Enabled);
    assert_eq!(again.parts, applied.parts);
    assert!(again.halted.is_none());
    assert_eq!(again.changed_at, 500);
    assert_eq!(again.ledger, applied.ledger);

    // With every part switched off, enable switches on what is available.
    let none_on = set_part(
        Some(&surface_only),
        Part::Surface,
        false,
        WINDOWS_PARTS,
        150,
    )
    .unwrap();
    assert!(!none_on.parts.any());
    let refilled = enable(Some(&none_on), None, WINDOWS_PARTS, 160).unwrap();
    assert_eq!(refilled.parts, WINDOWS_PARTS);

    // Parts: absent refuses, an unavailable part cannot be switched on.
    assert_eq!(
        set_part(None, Part::Surface, true, WINDOWS_PARTS, 1),
        Err(StateError::NotAdded)
    );
    assert_eq!(
        set_part(Some(&linux), Part::JumpList, true, LINUX_PARTS, 1),
        Err(StateError::PartUnavailable(Part::JumpList))
    );
    let off = set_part(Some(&linux), Part::Surface, false, LINUX_PARTS, 7).unwrap();
    assert!(!off.parts.surface);
    assert_eq!(
        set_part(Some(&off), Part::Surface, false, LINUX_PARTS, 8),
        Ok(off.clone())
    );

    // Resume at GUI start (C5.6) and the crash-loop halt per version (Ä30).
    let mut resumable = enabled.clone();
    assert!(resumable.should_resume("0.5.183"));
    resumable.halt("0.5.183", 600);
    assert!(!resumable.should_resume("0.5.183"));
    assert!(resumable.should_resume("0.5.184"));
    assert!(resumable.clear_halt());
    assert!(!resumable.clear_halt());
    assert!(resumable.should_resume("0.5.183"));
    assert!(!disabled.should_resume("0.5.183"));
    assert!(!none_on.should_resume("0.5.183"));

    for part in Part::ALL {
        assert_eq!(Part::parse_cli(part.cli_name()), Some(part));
    }
    assert_eq!(Part::parse_cli("nav_pane"), None);
    assert_eq!(
        [State::Absent, State::Enabled, State::Disabled].map(State::key),
        ["absent", "enabled", "disabled"]
    );
}

#[test]
fn desktop_task_state_schema_roundtrip() {
    let text = format!(
        r#"{{
  "schema": 1,
  "state": "enabled",
  "parts": {{ "surface": true, "nav_pane": true, "jump_list": false }},
  "changed_at": 1792000000,
  "halted": {{ "reason": "crash_loop", "version": "0.5.183", "at": 1792000500 }},
  "ledger": [
    {{ "kind": "run_value", "target": "{RUN_VALUE_NAME}", "applied_at": 1792000001 }},
    {{ "kind": "autostart_file", "target": "{AUTOSTART_FILE_NAME}", "applied_at": 1792000001 }},
    {{ "kind": "nav_pane_clsid", "target": "{DESKTOP_NAV_CLSID}", "applied_at": 1792000001 }},
    {{ "kind": "jump_list", "target": "{JUMP_LIST_TARGET}", "applied_at": 1792000001 }},
    {{ "kind": "gnome_extension_files", "target": "{GNOME_EXTENSION_UUID}", "applied_at": 1792000001 }},
    {{ "kind": "gnome_extension_enabled", "target": "{GNOME_EXTENSION_UUID}", "applied_at": 1792000001 }},
    {{ "kind": "foreign_extension_disabled", "target": "ding@rastersoft.com", "applied_at": 1792000001 }}
  ]
}}"#
    );
    let state = ModuleState::from_json(&text).unwrap();
    assert_eq!(
        state.halted.as_ref().map(|halt| halt.version.as_str()),
        Some("0.5.183")
    );
    let written = state.to_json().unwrap();
    assert_eq!(ModuleState::from_json(&written), Ok(state.clone()));
    let value: Value = serde_json::from_str(&written).unwrap();
    let original: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value, original);
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["changed_at", "halted", "ledger", "parts", "schema", "state"]
    );
    let kinds: Vec<&str> = value["ledger"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, Registration::ALL.map(Registration::key));

    // A new state writes `halted: null` and the Linux parts.
    let fresh = enable(None, None, LINUX_PARTS, 5).unwrap();
    let fresh: Value = serde_json::from_str(&fresh.to_json().unwrap()).unwrap();
    assert_eq!(fresh["halted"], Value::Null);
    assert_eq!(
        fresh["parts"],
        json!({ "surface": true, "nav_pane": false, "jump_list": false })
    );

    // Refused: another schema, a stored `absent`, an unknown halt reason.
    assert_eq!(
        ModuleState::from_json(&text.replace("\"schema\": 1", "\"schema\": 2")),
        Err(StateError::Schema(2))
    );
    assert_eq!(
        ModuleState::from_json(&text.replace("\"state\": \"enabled\"", "\"state\": \"absent\"")),
        Err(StateError::StoredAbsent)
    );
    assert!(matches!(
        ModuleState::from_json(&text.replace("crash_loop", "unbekannt")),
        Err(StateError::Json(_))
    ));
    // The draft field `mode` (Ä1) is ignored on read and never written.
    let with_mode = text.replacen(
        "\"schema\": 1,",
        "\"schema\": 1, \"mode\": \"embedded\",",
        1,
    );
    let read = ModuleState::from_json(&with_mode).unwrap();
    assert_eq!(read, state);
    assert!(!read.to_json().unwrap().contains("mode"));
}

#[test]
fn desktop_task_ledger_plans_known_traces_without_ledger() {
    assert_eq!(
        removal_plan(Platform::Windows, &[]),
        [
            trace(Registration::RunValue, "SmartExplorerDesktop"),
            trace(Registration::NavPaneClsid, DESKTOP_NAV_CLSID),
            trace(Registration::JumpList, "Smart Explorer.exe"),
        ]
    );
    assert_eq!(
        removal_plan(Platform::Linux, &[]),
        [
            trace(
                Registration::AutostartFile,
                "smart-explorer-desktop.desktop"
            ),
            trace(
                Registration::GnomeExtensionEnabled,
                "se-desktop@smartexplorer.app"
            ),
            trace(
                Registration::GnomeExtensionFiles,
                "se-desktop@smartexplorer.app"
            ),
        ]
    );
    // A full ledger adds nothing twice; other platforms' entries are ignored.
    let ledger: Vec<LedgerEntry> = KNOWN_TRACES
        .iter()
        .map(|(kind, target)| entry(*kind, target))
        .collect();
    for platform in [Platform::Windows, Platform::Linux] {
        assert_eq!(removal_plan(platform, &ledger), removal_plan(platform, &[]));
    }
    // A ledger target beside the fixed name (a path) is removed as well.
    let path = "/home/nutzer/.config/autostart/smart-explorer-desktop.desktop";
    let plan = removal_plan(Platform::Linux, &[entry(Registration::AutostartFile, path)]);
    assert_eq!(plan.len(), 4);
    assert!(plan.contains(&trace(Registration::AutostartFile, path)));
    // The CLSID is fixed and in registry form.
    assert_eq!(DESKTOP_NAV_CLSID.len(), 38);
    assert!(DESKTOP_NAV_CLSID.starts_with('{') && DESKTOP_NAV_CLSID.ends_with('}'));
    assert!(KNOWN_TRACES
        .iter()
        .all(|(kind, _)| *kind != Registration::ForeignExtensionDisabled));
}

#[test]
fn desktop_task_ledger_foreign_extension_only_with_entry() {
    let ding = "ding@rastersoft.com";
    let foreign = trace(Registration::ForeignExtensionDisabled, ding);
    assert!(!removal_plan(Platform::Linux, &[]).contains(&foreign));

    let ledger = [entry(Registration::ForeignExtensionDisabled, ding)];
    let plan = removal_plan(Platform::Linux, &ledger);
    // The own extension goes first, then the foreign one is switched back on.
    assert_eq!(plan.last(), Some(&foreign));
    let own = plan
        .iter()
        .position(|step| step.kind == Registration::GnomeExtensionEnabled)
        .unwrap();
    assert!(own + 1 < plan.len());
    assert!(removal_plan(Platform::Windows, &ledger)
        .iter()
        .all(|step| step.kind.platform() == Platform::Windows));

    // The entry is recorded once, survives `disable`, and is forgotten only
    // after the os layer switched the extension back on.
    let mut state = enable(None, None, LINUX_PARTS, 1).unwrap();
    state.record(ledger[0].clone());
    state.record(ledger[0].clone());
    assert_eq!(state.ledger.len(), 1);
    let disabled = disable(Some(&state), 2).unwrap();
    assert_eq!(
        removal_plan(Platform::Linux, &disabled.ledger).last(),
        Some(&foreign)
    );
    state.forget(Registration::ForeignExtensionDisabled, ding);
    assert!(state.ledger.is_empty());
}
