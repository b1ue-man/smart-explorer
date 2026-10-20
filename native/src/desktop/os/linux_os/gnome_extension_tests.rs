//! DSK-M8 signals that need no GNOME Shell: `desktop_task_gnome_extension_static`
//! (metadata, syntax, licence head and the rules of R36 for both variants),
//! `desktop_task_gnome_extension_install_remove` (installation, the user's
//! switches and the complete removal, with a stand-in `gsettings` and a
//! stand-in Shell), `desktop_task_gnome_ding_takeover_and_restore` (C2.5) and
//! `desktop_task_gnome_status_states` (U34-18).
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use super::gnome_control::{ExtensionReport, ExtensionSurface, Halted};
use super::gnome_dbus::STATE_ENABLED;
use super::gnome_extension::*;
use super::gnome_settings::{DISABLED, ENABLED};
use super::gnome_test_support::{FakeShell, Store};
use super::integration::{LinuxIntegration, LinuxPaths};
use super::session_facts::DESKTOP_ICON_EXTENSIONS;
use super::task_fixtures;
use crate::desktop::core::ledger::{removal_plan, Registration, Trace, GNOME_EXTENSION_UUID};
use crate::desktop::core::mechanism::GnomeVariant;
use crate::desktop::core::state::Platform;
use crate::desktop::os::control::Integration;

const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/gnome-extension");

fn read_asset(variant: &str, name: &str) -> String {
    std::fs::read_to_string(Path::new(ASSETS).join(variant).join(name))
        .unwrap_or_else(|error| panic!("{variant}/{name}: {error}"))
}

/// `node --check` on a copy named `.mjs`; `None` without node.
fn node_check(text: &str, dir: &Path, name: &str) -> Option<Result<(), String>> {
    let file = dir.join(format!("{name}.mjs"));
    std::fs::write(&file, text).unwrap();
    match Command::new("node").arg("--check").arg(&file).output() {
        Ok(output) if output.status.success() => Some(Ok(())),
        Ok(output) => Some(Err(String::from_utf8_lossy(&output.stderr).into_owned())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => Some(Err(error.to_string())),
    }
}

#[test]
fn desktop_task_gnome_extension_static() {
    let dir = tempfile::tempdir().unwrap();
    let mut checked_syntax = true;
    for (name, variant, versions) in [
        ("a", GnomeVariant::A, ["45", "46", "47", "48"].as_slice()),
        ("b", GnomeVariant::B, ["49", "50", "51"].as_slice()),
    ] {
        let metadata: serde_json::Value =
            serde_json::from_str(&read_asset(name, "metadata.json")).unwrap();
        assert_eq!(metadata["uuid"], GNOME_EXTENSION_UUID);
        assert_eq!(metadata["name"], "se Desktop");
        assert_eq!(metadata["version"], 1);
        assert_eq!(metadata["shell-version"], serde_json::json!(versions));
        assert!(metadata["description"]
            .as_str()
            .is_some_and(|text| !text.is_empty()));
        // The version lists and `variant_for` say the same.
        for major in 40..=60u32 {
            let listed = versions.contains(&major.to_string().as_str());
            assert_eq!(variant_for(major) == Some(variant), listed, "Shell {major}");
        }
        // What the program embeds is the file.
        let embedded = bundle(variant);
        assert_eq!(embedded.len(), 2);
        for (file, text) in embedded {
            assert_eq!(*text, read_asset(name, file), "{name}/{file}");
        }

        let script = read_asset(name, "extension.js");
        assert!(
            script.lines().count() < 250,
            "{name}: {} lines",
            script.lines().count()
        );
        assert!(
            script.starts_with("// SPDX-License-Identifier: MIT\n// Copyright (c)"),
            "MIT head"
        );
        assert!(script.contains("export default class SeDesktop extends Extension"));
        assert!(script.contains(&format!("const VARIANT = '{name}';")));
        assert!(script.contains("desktop-gnome.json") && script.contains("desktop-extension.json"));
        assert!(script.contains("'--gnome-monitor'") && script.contains("'--desktop-surface'"));
        // Only the windows of the extension's own clients are touched (R36):
        // no match by application, title or class.
        for forbidden in [
            "get_title",
            "get_wm_class",
            "get_gtk_application_id",
            "get_sandboxed_app_id",
            "app_id",
            "app.smartexplorer",
            "notify::title",
        ] {
            assert!(!script.contains(forbidden), "{name}: {forbidden}");
        }
        // No identifier or code of the foreign desktop-icons extensions.
        let lowered = script.to_lowercase();
        for foreign in [
            "ding@",
            "rastersoft",
            "smedius",
            "desktop-icons",
            "gtk4-ding",
        ] {
            assert!(!lowered.contains(foreign), "{name}: {foreign}");
        }
        match variant {
            GnomeVariant::A => {
                for needed in [
                    "query_window_belongs_to",
                    "hide_from_window_list",
                    "window.lower()",
                    "window.stick()",
                    "'restacked'",
                    "Meta.WaylandClient.new(",
                ] {
                    assert!(script.contains(needed), "a: {needed}");
                }
            }
            GnomeVariant::B => {
                for needed in [
                    "owns_window",
                    "window.set_type(Meta.WindowType.DESKTOP)",
                    "window.stick()",
                    "Meta.WaylandClient.new_subprocess(",
                ] {
                    assert!(script.contains(needed), "b: {needed}");
                }
                assert!(!script.contains("lower()"), "b does not lower");
            }
        }
        for needed in [
            "move_resize_frame(false",
            "'window-created'",
            "send_signal(15)",
        ] {
            assert!(script.contains(needed), "{name}: {needed}");
        }
        match node_check(&script, dir.path(), name) {
            Some(Ok(())) => {}
            Some(Err(error)) => panic!("{name}/extension.js: node --check: {error}"),
            None => checked_syntax = false,
        }
    }
    assert!(
        checked_syntax,
        "nicht ausgeführt: node fehlt (ESM-Syntaxprüfung der Erweiterung)"
    );
}

struct Setup {
    root: tempfile::TempDir,
    paths: LinuxPaths,
    store: Arc<Store>,
}

fn setup() -> Setup {
    let root = tempfile::tempdir().unwrap();
    let paths = task_fixtures::paths(root.path());
    let store = Arc::new(Store::new(root.path()));
    Setup { root, paths, store }
}

fn kinds(report: &InstallReport) -> Vec<&'static str> {
    report
        .entries
        .iter()
        .map(|entry| entry.kind.key())
        .collect()
}

#[test]
fn desktop_task_gnome_extension_install_remove() {
    let setup = setup();
    let dir = extension_dir(&setup.paths);
    let before = task_fixtures::trace_snapshot(setup.root.path());
    setup.store.set_list(ENABLED, &["foreign@example.org"]);
    let gnome = setup.store.settings();

    // The running Shell does not know the new extension: after the next login.
    let shell = FakeShell::new(&setup.store, "46.0", false);
    let report = install(&setup.paths, &gnome, &*shell.bus(), GnomeVariant::A).unwrap();
    assert_eq!(report.outcome, InstallOutcome::NeedsRelogin);
    assert_eq!(
        kinds(&report),
        ["gnome_extension_files", "gnome_extension_enabled"]
    );
    assert!(files_current(&dir, GnomeVariant::A));
    assert!(!files_current(&dir, GnomeVariant::B));
    assert_eq!(
        setup.store.list(ENABLED),
        ["foreign@example.org", GNOME_EXTENSION_UUID],
        "the UUID joins the list and the foreign entry stays"
    );
    assert_eq!(shell.calls(), [format!("enable {GNOME_EXTENSION_UUID}")]);

    // Changed code is written again (the SHA-256 differs), unchanged code not.
    std::fs::write(dir.join("extension.js"), "// changed\n").unwrap();
    assert!(!files_current(&dir, GnomeVariant::A));
    install(&setup.paths, &gnome, &*shell.bus(), GnomeVariant::A).unwrap();
    assert!(files_current(&dir, GnomeVariant::A));
    assert_eq!(setup.store.list(ENABLED).len(), 2, "no second entry");
    // A Shell with another major version takes the other variant.
    install(&setup.paths, &gnome, &*shell.bus(), GnomeVariant::B).unwrap();
    assert!(files_current(&dir, GnomeVariant::B));

    // A Shell that knows it (it started after the first installation).
    let running = FakeShell::new(&setup.store, "46.0", true);
    let report = install(&setup.paths, &gnome, &*running.bus(), GnomeVariant::A).unwrap();
    assert_eq!(report.outcome, InstallOutcome::Enabled);
    assert!(
        install(&setup.paths, &gnome, &*running.bus(), GnomeVariant::A).is_ok(),
        "again: the Shell lists it as enabled, no second call"
    );
    assert_eq!(running.calls().len(), 1);

    // The user's switches are reported and never changed.
    setup.store.set_list(DISABLED, &[GNOME_EXTENSION_UUID]);
    setup.store.set_list(ENABLED, &["foreign@example.org"]);
    let untouched = FakeShell::new(&setup.store, "46.0", true);
    let report = install(&setup.paths, &gnome, &*untouched.bus(), GnomeVariant::A).unwrap();
    assert_eq!(report.outcome, InstallOutcome::DisabledByUser);
    assert_eq!(kinds(&report), ["gnome_extension_files"]);
    assert_eq!(setup.store.list(ENABLED), ["foreign@example.org"]);
    assert_eq!(setup.store.list(DISABLED), [GNOME_EXTENSION_UUID]);
    assert!(untouched.calls().is_empty());
    setup.store.set_list(DISABLED, &[]);
    setup.store.set_bool("disable-user-extensions", true);
    let report = install(&setup.paths, &gnome, &*untouched.bus(), GnomeVariant::A).unwrap();
    assert_eq!(report.outcome, InstallOutcome::UserExtensionsDisabled);
    assert_eq!(setup.store.list(ENABLED), ["foreign@example.org"]);
    assert!(untouched.calls().is_empty());
    setup.store.set_bool("disable-user-extensions", false);

    // No gsettings at all: only the files.
    let none = super::gnome_settings::GnomeSettings::none();
    let report = install(&setup.paths, &none, &*untouched.bus(), GnomeVariant::A).unwrap();
    assert_eq!(report.outcome, InstallOutcome::NoSettings);

    // Removing: through the integration and the removal plan, as `disable`
    // and `remove` do; the foreign entry stays, nothing of se stays.
    let _ = install(&setup.paths, &gnome, &*running.bus(), GnomeVariant::A).unwrap();
    assert!(setup
        .store
        .list(ENABLED)
        .contains(&GNOME_EXTENSION_UUID.to_string()));
    let mut integration =
        LinuxIntegration::with(setup.paths.clone(), setup.store.settings(), running.bus());
    let plan = removal_plan(Platform::Linux, &[]);
    for trace in &plan {
        integration.unapply(trace).unwrap();
        assert!(!integration.is_applied(trace), "{trace:?}");
    }
    assert!(!dir.exists());
    assert_eq!(setup.store.list(ENABLED), ["foreign@example.org"]);
    assert!(setup.store.list(DISABLED).is_empty());
    assert!(
        running
            .calls()
            .contains(&format!("disable {GNOME_EXTENSION_UUID}")),
        "the running Shell is told"
    );
    let after = task_fixtures::trace_snapshot(setup.root.path());
    assert_eq!(before, after, "no file of se is left");
    // A foreign extension under the same folder is not ours to remove.
    let foreign = Trace {
        kind: Registration::GnomeExtensionFiles,
        target: "other@example.org".into(),
    };
    assert!(integration.unapply(&foreign).is_err());
}

/// A DING that is enabled, its folder, and the Shell that loaded it.
fn with_ding(setup: &Setup, uuid: &str) -> Arc<FakeShell> {
    let folder = setup.paths.extensions_dir.join(uuid);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("metadata.json"), "{}").unwrap();
    setup
        .store
        .set_list(ENABLED, &[uuid, "foreign@example.org"]);
    let shell = FakeShell::new(&setup.store, "46.0", false);
    shell.load(uuid);
    shell
}

#[test]
fn desktop_task_gnome_ding_takeover_and_restore() {
    // The names that are looked for.
    assert_eq!(
        DESKTOP_ICON_EXTENSIONS,
        ["ding@rastersoft.com", "gtk4-ding@smedius.gitlab.com"]
    );
    for uuid in DESKTOP_ICON_EXTENSIONS {
        let setup = setup();
        let shell = with_ding(&setup, uuid);
        // `take_over`: DisableExtension and a ledger entry with the UUID.
        let entry = take_over(&*shell.bus(), uuid).unwrap();
        assert_eq!(entry.kind, Registration::ForeignExtensionDisabled);
        assert_eq!(entry.target, uuid);
        assert_eq!(shell.calls(), [format!("disable {uuid}")]);
        assert_eq!(setup.store.list(ENABLED), ["foreign@example.org"]);
        // An unknown or already disabled extension is no take-over.
        assert!(take_over(&*shell.bus(), "unknown@example.org").is_err());

        // `remove`/`disable` with the ledger entry: switched on again.
        let mut integration =
            LinuxIntegration::with(setup.paths.clone(), setup.store.settings(), shell.bus());
        let trace = Trace {
            kind: Registration::ForeignExtensionDisabled,
            target: uuid.to_string(),
        };
        assert!(integration.is_applied(&trace), "still off and installed");
        integration.unapply(&trace).unwrap();
        assert!(setup.store.list(ENABLED).contains(&uuid.to_string()));
        assert!(shell.calls().contains(&format!("enable {uuid}")));
        assert!(!integration.is_applied(&trace));
    }

    // The user switched DING on again meanwhile: nothing is called twice and
    // the list keeps one entry.
    let setup_on = setup();
    let ding = DESKTOP_ICON_EXTENSIONS[0];
    let shell = with_ding(&setup_on, ding);
    let mut integration = LinuxIntegration::with(
        setup_on.paths.clone(),
        setup_on.store.settings(),
        shell.bus(),
    );
    let trace = Trace {
        kind: Registration::ForeignExtensionDisabled,
        target: ding.to_string(),
    };
    integration.unapply(&trace).unwrap();
    assert!(shell.calls().is_empty(), "DING was on: left alone");
    assert_eq!(setup_on.store.list(ENABLED), [ding, "foreign@example.org"]);

    // The user removed DING meanwhile: it does not come back.
    let setup_gone = setup();
    let shell = with_ding(&setup_gone, ding);
    take_over(&*shell.bus(), ding).unwrap();
    std::fs::remove_dir_all(setup_gone.paths.extensions_dir.join(ding)).unwrap();
    let mut integration = LinuxIntegration::with(
        setup_gone.paths.clone(),
        setup_gone.store.settings(),
        shell.bus(),
    );
    integration.unapply(&trace).unwrap();
    assert_eq!(setup_gone.store.list(ENABLED), ["foreign@example.org"]);

    // Without a ledger entry the removal plan never names a foreign
    // extension at all (C5.7).
    let plan = removal_plan(Platform::Linux, &[]);
    assert!(plan
        .iter()
        .all(|trace| trace.kind != Registration::ForeignExtensionDisabled));
}

fn inputs() -> StateInputs {
    StateInputs {
        applicable: true,
        shell_major: Some(46),
        files_present: true,
        user_extensions_disabled: false,
        in_disabled_list: false,
        in_enabled_list: true,
        shell_runs_it: Some(true),
        report: None,
    }
}

fn report(handled: bool, halted: bool) -> ExtensionReport {
    ExtensionReport {
        version: 1,
        variant: "a".into(),
        surfaces: vec![ExtensionSurface {
            monitor: 0,
            pid: 7,
            handled,
        }],
        halted: halted.then(|| Halted {
            reason: "crash_loop".into(),
            at: 5,
        }),
    }
}

#[test]
fn desktop_task_gnome_status_states() {
    let state = |change: &dyn Fn(&mut StateInputs)| {
        let mut value = inputs();
        change(&mut value);
        extension_state(&value).map(ExtensionState::key)
    };
    // Outside the extension's session: no statement at all.
    assert_eq!(state(&|i| i.applicable = false), None);
    assert_eq!(state(&|i| i.shell_major = None), None);
    // Shell versions without a variant.
    assert_eq!(
        state(&|i| i.shell_major = Some(44)),
        Some("unsupported_shell_version")
    );
    assert_eq!(
        state(&|i| i.shell_major = Some(52)),
        Some("unsupported_shell_version")
    );
    // Installed, enabled, loaded, no window placed yet.
    assert_eq!(state(&|_| {}), Some("installed"));
    // Written and enabled, but the running Shell does not know it.
    assert_eq!(
        state(&|i| i.shell_runs_it = Some(false)),
        Some("installed_needs_relogin")
    );
    // The bus did not answer: no claim about a login.
    assert_eq!(state(&|i| i.shell_runs_it = None), Some("installed"));
    // Not enabled in the settings: installed, nothing to relogin for.
    assert_eq!(
        state(&|i| {
            i.in_enabled_list = false;
            i.shell_runs_it = Some(false);
        }),
        Some("installed")
    );
    assert_eq!(state(&|i| i.files_present = false), Some("not_installed"));
    // The user's switches win over the files.
    assert_eq!(
        state(&|i| i.in_disabled_list = true),
        Some("disabled_by_user")
    );
    assert_eq!(
        state(&|i| i.user_extensions_disabled = true),
        Some("user_extensions_disabled")
    );
    assert_eq!(
        state(&|i| {
            i.user_extensions_disabled = true;
            i.in_disabled_list = true;
        }),
        Some("user_extensions_disabled")
    );
    // The extension's own report.
    assert_eq!(
        state(&|i| i.report = Some(report(true, false))),
        Some("running")
    );
    assert_eq!(
        state(&|i| i.report = Some(report(false, false))),
        Some("installed")
    );
    assert_eq!(
        state(&|i| i.report = Some(report(true, true))),
        Some("halted")
    );
    assert_eq!(
        state(&|i| {
            i.report = Some(report(true, true));
            i.in_disabled_list = true;
        }),
        Some("halted"),
        "a halt is reported first"
    );
    // The eight keys are the contract.
    let keys: Vec<&str> = [
        ExtensionState::NotInstalled,
        ExtensionState::Installed,
        ExtensionState::InstalledNeedsRelogin,
        ExtensionState::Running,
        ExtensionState::DisabledByUser,
        ExtensionState::UserExtensionsDisabled,
        ExtensionState::UnsupportedShellVersion,
        ExtensionState::Halted,
    ]
    .iter()
    .map(|state| state.key())
    .collect();
    assert_eq!(
        keys,
        [
            "not_installed",
            "installed",
            "installed_needs_relogin",
            "running",
            "disabled_by_user",
            "user_extensions_disabled",
            "unsupported_shell_version",
            "halted"
        ]
    );
}
