//! Agent plugin repair of 2026-11-14: a daemon started from an app copy outside every
//! installation (cause U1) answers a runtime for the installation the runtime
//! names, after the same checks `Installation::load` applies; and an elevated
//! helper refreshes the agent profiles of an installation its own account
//! owns (cause U3), and only below directories of that account.
use super::batches_lifecycle_task_tests::Fixture;
use crate::mcp_launcher::core::manifest::{IPC_CONTRACT, STATE_CONTRACT};
use crate::mcp_launcher::installation::refresh_stays_in_own_account;
use crate::mcp_launcher::{
    profile_refresh_deferred, Installation, RuntimeContract, RuntimeHello,
    DAEMON_OUTSIDE_INSTALLATION,
};

fn root_text(fixture: &Fixture) -> String {
    fixture.installation.root.to_string_lossy().into_owned()
}

#[test]
fn plugin_task_daemon_outside_an_installation_admits_the_named_installation() {
    let fixture = Fixture::new();
    let id = fixture.installation.manifest.installation_id.clone();
    let root = root_text(&fixture);
    let admitted = Installation::for_daemon(Ok(None), &id, Some(root.as_str())).unwrap();
    assert_eq!(admitted.root, fixture.installation.root);
    assert!(admitted.manifest.permits(&RuntimeContract {
        runtime_sha256: fixture.installation.manifest.runtime_sha256.clone(),
        ipc_contract_id: IPC_CONTRACT.into(),
        shared_state_contract_id: STATE_CONTRACT.into(),
    }));
}

#[test]
fn plugin_task_daemon_outside_an_installation_asks_for_the_root() {
    let fixture = Fixture::new();
    let id = fixture.installation.manifest.installation_id.clone();
    assert_eq!(
        Installation::for_daemon(Ok(None), &id, None).unwrap_err(),
        DAEMON_OUTSIDE_INSTALLATION
    );
}

#[test]
fn plugin_task_daemon_refuses_a_named_root_of_another_installation() {
    let fixture = Fixture::new();
    let root = root_text(&fixture);
    assert_eq!(
        Installation::for_daemon(Ok(None), "another-installation", Some(root.as_str()))
            .unwrap_err(),
        "runtime_installation_id_mismatch"
    );
    let missing = fixture.installation.root.join("missing");
    assert!(Installation::for_daemon(
        Ok(None),
        &fixture.installation.manifest.installation_id,
        Some(missing.to_string_lossy().as_ref()),
    )
    .is_err());
}

#[test]
fn plugin_task_daemon_refuses_a_relative_root_and_an_installation_being_removed() {
    let fixture = Fixture::new();
    let id = fixture.installation.manifest.installation_id.clone();
    // A runtime names the canonical root of its image; a relative path is none.
    assert_eq!(
        Installation::for_daemon(Ok(None), &id, Some("mcp/..")).unwrap_err(),
        "runtime_installation_root_invalid"
    );
    // As a daemon inside an installation that is being removed refuses its runtimes.
    let root = root_text(&fixture);
    assert!(Installation::for_daemon(Ok(None), &id, Some(root.as_str())).is_ok());
    std::fs::write(
        fixture.installation.root.join("mcp/uninstall.v1.json"),
        b"{}",
    )
    .unwrap();
    assert_eq!(
        Installation::for_daemon(Ok(None), &id, Some(root.as_str())).unwrap_err(),
        "installation_uninstall_in_progress"
    );
}

#[test]
fn plugin_task_daemon_of_an_installation_answers_only_for_its_own() {
    let fixture = Fixture::new();
    let id = fixture.installation.manifest.installation_id.clone();
    let own = Installation::for_daemon(
        Ok(Some(fixture.installation.clone())),
        &id,
        Some("/a/root/the/runtime/named"),
    )
    .unwrap();
    assert_eq!(own.root, fixture.installation.root);
    assert_eq!(
        Installation::for_daemon(
            Ok(Some(fixture.installation.clone())),
            "another-installation",
            Some(root_text(&fixture).as_str()),
        )
        .unwrap_err(),
        "runtime_installation_id_mismatch"
    );
    assert_eq!(
        Installation::for_daemon(
            Err("image unreadable".into()),
            &id,
            Some(root_text(&fixture).as_str())
        )
        .unwrap_err(),
        "image unreadable"
    );
}

#[test]
fn plugin_task_runtime_hello_without_root_keeps_the_previous_wire_form() {
    let hello = RuntimeHello {
        installation_id: "i".into(),
        runtime_sha256: "r".into(),
        ipc_contract_id: IPC_CONTRACT.into(),
        shared_state_contract_id: STATE_CONTRACT.into(),
        installation_root: None,
    };
    let plain = serde_json::to_value(&hello).unwrap();
    assert!(plain.get("installation_root").is_none());
    assert_eq!(plain.as_object().unwrap().len(), 4);
    let old: RuntimeHello = serde_json::from_value(plain).unwrap();
    assert!(old.installation_root.is_none());
    let named = RuntimeHello {
        installation_root: Some("/opt/se".into()),
        ..hello
    };
    let named = serde_json::to_value(&named).unwrap();
    assert_eq!(named["installation_root"], "/opt/se");
}

#[test]
fn plugin_task_elevated_helper_refreshes_only_its_own_installation() {
    let fixture = Fixture::new();
    let root = &fixture.installation.root;
    assert!(!profile_refresh_deferred(root, false));
    assert!(profile_refresh_deferred(&root.join("missing"), true));
    // The fixture belongs to the account running the test; a home directory of
    // that account too. Windows never tells the accounts apart and defers.
    let own_home = Some(root.clone());
    assert_eq!(
        refresh_stays_in_own_account(root, std::slice::from_ref(&own_home)),
        !cfg!(windows)
    );
    // Root's helper started with another account's home (`sudo -E`) would
    // write that account's profiles: no home, a missing one and a home of
    // another account keep the refresh deferred.
    assert!(!refresh_stays_in_own_account(
        root,
        &[own_home.clone(), None]
    ));
    assert!(!refresh_stays_in_own_account(
        root,
        &[own_home.clone(), Some(root.join("missing"))]
    ));
    assert!(!refresh_stays_in_own_account(
        &root.join("missing"),
        &[own_home.clone()]
    ));
    #[cfg(target_os = "linux")]
    {
        // `/` belongs to root: another account's directory for every test
        // account but root itself.
        let foreign = Some(std::path::PathBuf::from("/"));
        let own_and_foreign = refresh_stays_in_own_account(root, &[own_home, foreign]);
        assert_eq!(own_and_foreign, unsafe { libc::geteuid() } == 0);
    }
}
