//! Host tests of the Share settings added to the Android facade in the GUI
//! parity batch: the exec target and provider fields, the program list of
//! `share.setExec`, and the argument rules of the new setting methods.
use serde_json::json;

use super::share_device_settings::{contact_options, export_offset};
use super::share_exec::{program_settings, provider_json, targets_json};
use crate::share::{
    DirectGrant, DirectGrantState, ExecGrant, ExecProviderStatus, ExecRunAs, ShareProfiles,
};

#[test]
fn luecken_task_bridge_exec_targets_carry_account_and_program_list() {
    let mut profiles = ShareProfiles::default();
    profiles.direct_grants.push(DirectGrant {
        device_id: "d-laptop".into(),
        device_name: "Laptop".into(),
        public_key: "key".into(),
        fingerprint: "fp".into(),
        node_id: "node".into(),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: ExecGrant {
            enabled: true,
            policy_revision: 2,
            programs: vec!["uptime".into()],
            ..ExecGrant::default()
        },
        write: false,
    });
    let targets = targets_json(&profiles);
    assert_eq!(targets[0]["runAs"], "standard");
    assert_eq!(targets[0]["programs"], json!(["uptime"]));
    let provider = provider_json(&ExecProviderStatus {
        available: true,
        provider: "android-subreaper".into(),
        detail: String::new(),
        elevated: false,
        user_label: "u0_a123".into(),
    });
    assert_eq!(provider["userLabel"], "u0_a123");
    assert_eq!(
        provider["runAsSupported"],
        crate::share::exec_run_as_supported()
    );
}

#[test]
fn luecken_task_bridge_set_exec_programs_follow_the_cli_rules() {
    assert_eq!(program_settings(&json!({})).unwrap(), None);
    assert_eq!(
        program_settings(&json!({ "programs": null })).unwrap(),
        None
    );
    let full = program_settings(&json!({ "programs": [] }))
        .unwrap()
        .unwrap();
    assert!(full.programs.is_empty());
    assert_eq!(full.run_as, ExecRunAs::Standard);
    let listed = program_settings(&json!({ "programs": [" uptime ", "/system/bin/df"] }))
        .unwrap()
        .unwrap();
    assert_eq!(listed.programs, ["uptime", "/system/bin/df"]);
    for refused in [
        json!({ "programs": ["bin/tool"] }),
        json!({ "programs": [""] }),
        json!({ "programs": "uptime" }),
        json!({ "programs": [1] }),
    ] {
        assert_eq!(
            program_settings(&refused).expect_err("refused").kind,
            "invalid",
            "{refused}"
        );
    }
}

#[test]
fn luecken_task_bridge_setting_arguments_are_checked_before_any_change() {
    assert_eq!(
        contact_options(&json!({ "autoConnect": false })).unwrap(),
        (Some(false), None)
    );
    assert_eq!(
        contact_options(&json!({ "autoOpen": true, "autoConnect": null })).unwrap(),
        (None, Some(true))
    );
    assert!(contact_options(&json!({})).is_err());
    assert!(contact_options(&json!({ "autoOpen": "yes" })).is_err());
    assert_eq!(export_offset(&json!({ "offset": -1 })).unwrap(), -1);
    assert_eq!(export_offset(&json!({ "offset": 1 })).unwrap(), 1);
    assert!(export_offset(&json!({ "offset": 2 })).is_err());
    assert!(export_offset(&json!({})).is_err());
}
