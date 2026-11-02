//! DSK-M5 checks of the Windows registry: the twelve values of the
//! navigation pane (C1.4) and their removal, the root following the S2
//! device tree through the shell's parts, and the start values of
//! "Papierkorb" and "Dieser PC" (Ä5). The navigation-pane checks change the
//! runner's HKCU and run only through the isolated remote desktop task
//! entrypoint (`SMART_EXPLORER_DESKTOP_TASK=1`, one test thread); an account
//! where the root already exists is refused, never changed.
use std::collections::BTreeMap;
use std::path::Path;

use winreg::enums::{RegType, HKEY_CURRENT_USER, KEY_ALL_ACCESS};
use winreg::{RegKey, RegValue};

use super::nav_pane::{self, Data, NavValue, CLSID_ROOT, HIDE_ICONS, NAMESPACE_ROOT};
use super::shell_parts::ShellParts;
use super::{system_items, PlatformIntegration};
use crate::desktop::core::config::SystemItems;
use crate::desktop::core::ledger::{Registration, Trace, DESKTOP_NAV_CLSID};
use crate::desktop::core::state::{ModuleState, Part, PartSet, State, STATE_SCHEMA};
use crate::desktop::os::control::Integration;
use crate::desktop::os::feed_sources::{DeviceTreeStatus, DeviceTreeView};
use crate::desktop::os::store::DesktopDir;

const GUI: &str = r"C:\Program Files\Smart Explorer\Smart Explorer.exe";

fn hkcu() -> RegKey {
    RegKey::predef(HKEY_CURRENT_USER)
}

fn require_task_entrypoint() {
    assert_eq!(
        std::env::var("SMART_EXPLORER_DESKTOP_TASK").as_deref(),
        Ok("1"),
        "isolated remote task entrypoint required"
    );
    assert!(
        !nav_pane::present(),
        "precondition: the navigation pane root {DESKTOP_NAV_CLSID} exists before the test"
    );
}

fn raw_value(key: &str, name: &str) -> Option<RegValue> {
    hkcu().open_subkey(key).ok()?.get_raw_value(name).ok()
}

fn key_exists(key: &str) -> bool {
    hkcu().open_subkey(key).is_ok()
}

/// The twelve rows of C1.4, written out.
fn expected_rows(root: &str) -> Vec<NavValue> {
    let clsid = format!(r"Software\Classes\CLSID\{DESKTOP_NAV_CLSID}");
    let row = |key: &str, name: &str, data: Data| NavValue {
        key: key.to_string(),
        name: name.to_string(),
        data,
    };
    vec![
        row(&clsid, "", Data::Sz("Smart Explorer Geräte".into())),
        row(
            &format!(r"{clsid}\DefaultIcon"),
            "",
            Data::ExpandSz(format!("\"{GUI}\",0")),
        ),
        row(&clsid, "System.IsPinnedToNameSpaceTree", Data::Dword(1)),
        row(&clsid, "SortOrderIndex", Data::Dword(0x42)),
        row(
            &format!(r"{clsid}\InProcServer32"),
            "",
            Data::ExpandSz(r"%systemroot%\system32\shell32.dll".into()),
        ),
        row(
            &format!(r"{clsid}\Instance"),
            "CLSID",
            Data::Sz("{0E5AAE11-A475-4c5b-AB00-C66DE400274E}".into()),
        ),
        row(
            &format!(r"{clsid}\Instance\InitPropertyBag"),
            "Attributes",
            Data::Dword(0x11),
        ),
        row(
            &format!(r"{clsid}\Instance\InitPropertyBag"),
            "TargetFolderPath",
            Data::ExpandSz(root.into()),
        ),
        row(
            &format!(r"{clsid}\ShellFolder"),
            "FolderValueFlags",
            Data::Dword(0x28),
        ),
        row(
            &format!(r"{clsid}\ShellFolder"),
            "Attributes",
            Data::Dword(0xF080_004D),
        ),
        row(
            &format!(
                r"Software\Microsoft\Windows\CurrentVersion\Explorer\Desktop\NameSpace\{DESKTOP_NAV_CLSID}"
            ),
            "",
            Data::Sz("Smart Explorer Geräte".into()),
        ),
        row(
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\NewStartPanel",
            DESKTOP_NAV_CLSID,
            Data::Dword(1),
        ),
    ]
}

/// The parent keys of the root, read now.
fn parents_now() -> Vec<bool> {
    [
        CLSID_ROOT,
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\Desktop",
        NAMESPACE_ROOT,
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons",
        HIDE_ICONS,
    ]
    .iter()
    .map(|key| key_exists(key))
    .collect()
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_windows_nav_pane_keys_match_spec() {
    require_task_entrypoint();
    let before = parents_now();
    let created = nav_pane::missing_parents();
    let rows = expected_rows(r"X:\");
    assert_eq!(nav_pane::spec(Path::new(GUI), r"X:\"), rows);
    nav_pane::apply(Path::new(GUI), r"X:\").unwrap();
    for row in &rows {
        let found = raw_value(&row.key, &row.name);
        assert_eq!(
            found.as_ref(),
            Some(&nav_pane::raw(&row.data)),
            "{} {:?}",
            row.key,
            row.name
        );
    }
    assert_eq!(nav_pane::current_target().as_deref(), Some(r"X:\"));
    // Removal through the integration, as `se desktop disable` runs it:
    // the root, then the parent keys the root created.
    let mut integration = PlatformIntegration::for_task(Path::new("."));
    let clsid = Trace {
        kind: Registration::NavPaneClsid,
        target: DESKTOP_NAV_CLSID.into(),
    };
    assert!(integration.is_applied(&clsid));
    for target in &created {
        let trace = Trace {
            kind: Registration::NavPaneClsid,
            target: target.clone(),
        };
        assert!(integration.is_applied(&trace), "{target}");
        integration.unapply(&trace).unwrap();
        assert!(!integration.is_applied(&trace), "{target}");
    }
    integration.unapply(&clsid).unwrap();
    assert!(!integration.is_applied(&clsid));
    for row in &rows {
        assert_eq!(raw_value(&row.key, &row.name), None, "{}", row.key);
    }
    assert_eq!(parents_now(), before);
    // A foreign key is never deleted.
    let foreign = Trace {
        kind: Registration::NavPaneClsid,
        target: r"HKCU\Software\Microsoft".into(),
    };
    assert!(integration.unapply(&foreign).is_err());
    assert!(key_exists(r"Software\Microsoft"));
}

fn write_state(dir: &DesktopDir, parts: &[Part]) -> ModuleState {
    let previous = dir.read_state().unwrap();
    let state = ModuleState {
        schema: STATE_SCHEMA,
        state: State::Enabled,
        parts: PartSet::of(parts),
        changed_at: 1,
        halted: None,
        ledger: previous.map(|state| state.ledger).unwrap_or_default(),
    };
    dir.write_state(&state).unwrap();
    state
}

fn mounted(root: Option<&str>) -> DeviceTreeView {
    DeviceTreeView::Known(DeviceTreeStatus {
        enabled: true,
        state: "mounted".into(),
        mounted: root.is_some(),
        root_path: root.map(str::to_string),
        revision: 1,
        paths: BTreeMap::new(),
    })
}

fn nav_entries(dir: &DesktopDir) -> Vec<String> {
    dir.read_state()
        .unwrap()
        .map(|state| state.ledger)
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| entry.kind == Registration::NavPaneClsid)
        .map(|entry| entry.target)
        .collect()
}

#[test]
#[ignore = "run only through the isolated remote desktop task entrypoint"]
fn desktop_task_windows_nav_pane_follows_device_root() {
    require_task_entrypoint();
    let before = parents_now();
    let created = nav_pane::missing_parents();
    let root = tempfile::tempdir().unwrap();
    let dir = DesktopDir::at(root.path().join("desktop"));
    dir.create().unwrap();
    let mut parts = ShellParts::new(dir.clone(), GUI.into());
    // Mounted: the root with the tree's path, ledger first.
    let state = write_state(&dir, &[Part::NavPane]);
    parts.sync(&state, &mounted(Some(r"X:\")), None);
    assert_eq!(nav_pane::current_target().as_deref(), Some(r"X:\"));
    let mut recorded = nav_entries(&dir);
    recorded.sort();
    let mut wanted = created.clone();
    wanted.push(DESKTOP_NAV_CLSID.into());
    wanted.sort();
    assert_eq!(recorded, wanted);
    // A new root rewrites the target.
    parts.sync(&state, &mounted(Some(r"Y:\")), None);
    assert_eq!(nav_pane::current_target().as_deref(), Some(r"Y:\"));
    // Not mounted: no root, no ledger entry, no created parent key.
    parts.sync(&state, &mounted(None), None);
    assert!(!nav_pane::present());
    assert!(nav_entries(&dir).is_empty());
    assert_eq!(parents_now(), before);
    // Mounted again, then a service that no longer runs: gone again.
    parts.sync(&state, &mounted(Some(r"X:\")), None);
    assert!(nav_pane::present());
    parts.sync(&state, &DeviceTreeView::NotRunning, None);
    assert!(!nav_pane::present());
    // The part switched off: never written although mounted.
    let off = write_state(&dir, &[Part::Surface]);
    parts.sync(&off, &mounted(Some(r"X:\")), None);
    assert!(!nav_pane::present());
    assert!(nav_entries(&dir).is_empty());
    assert_eq!(parents_now(), before);
}

#[test]
fn desktop_task_windows_system_items_initial_value() {
    assert_eq!(
        HIDE_ICONS,
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\HideDesktopIcons\NewStartPanel"
    );
    let parent = r"Software\SmartExplorerDesktopTask";
    let key = format!(r"{parent}\NewStartPanel-{}", std::process::id());
    let recycle = "{645FF040-5081-101B-9F08-00AA002F954E}";
    let this_pc = "{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
    let items = |recycle_bin, this_pc| SystemItems {
        recycle_bin,
        this_pc,
    };
    // No key, no value: the se default.
    assert_eq!(system_items::initial_from(&key), items(true, false));
    let (opened, _) = hkcu().create_subkey(&key).unwrap();
    assert_eq!(system_items::initial_from(&key), items(true, false));
    opened.set_value(recycle, &1u32).unwrap();
    opened.set_value(this_pc, &0u32).unwrap();
    assert_eq!(system_items::initial_from(&key), items(false, true));
    opened.set_value(recycle, &0u32).unwrap();
    opened.set_value(this_pc, &1u32).unwrap();
    assert_eq!(system_items::initial_from(&key), items(true, false));
    // A value of another type counts as missing.
    let text = RegValue {
        bytes: "1\0".encode_utf16().flat_map(u16::to_le_bytes).collect(),
        vtype: RegType::REG_SZ,
    };
    opened.set_raw_value(recycle, &text).unwrap();
    opened.delete_value(this_pc).unwrap();
    assert_eq!(system_items::initial_from(&key), items(true, false));
    drop(opened);
    hkcu().delete_subkey_all(&key).unwrap();
    if let Ok(parent_key) = hkcu().open_subkey_with_flags(parent, KEY_ALL_ACCESS) {
        let empty = parent_key
            .query_info()
            .is_ok_and(|info| info.sub_keys == 0 && info.values == 0);
        drop(parent_key);
        if empty {
            let _ = hkcu().delete_subkey(parent);
        }
    }
}
