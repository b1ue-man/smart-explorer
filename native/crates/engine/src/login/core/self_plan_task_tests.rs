//! E21 setup plan: one setup per version, removal, image binding.
use super::*;
use crate::login::core::machine::MachineIdentity;

const VERSION: &str = "0.5.197";

fn marker(state: SelfState, trigger: LoginTrigger) -> Marker {
    Marker {
        state,
        trigger,
        at: 1,
    }
}

fn running(release: &str) -> Installed {
    Installed::Running {
        release: release.into(),
    }
}

#[test]
fn login_selbst_task_version_change_sets_up_exactly_once() {
    // First start of a new version with an older service: one setup.
    let old = running("0.5.196");
    assert_eq!(
        decide(LoginTrigger::VersionStart, true, &old, None, VERSION),
        Plan::Install
    );
    // That run wrote the marker of this version: no second setup, also not
    // for a presence, a list change, a coverage change or the watch. The
    // older service that still runs is synced without a prompt; without a
    // service the result stays as it is.
    for state in [
        SelfState::Done,
        SelfState::Declined,
        SelfState::Failed("script_failed".into()),
    ] {
        let written = marker(state.clone(), LoginTrigger::VersionStart);
        for trigger in [
            LoginTrigger::VersionStart,
            LoginTrigger::PeerOnline,
            LoginTrigger::ListChanged,
            LoginTrigger::CoverageEnded,
            LoginTrigger::Watch,
        ] {
            assert_eq!(
                decide(trigger, true, &old, Some(&written), VERSION),
                Plan::SyncKeep(Some(state.clone()))
            );
            assert_eq!(
                decide(trigger, true, &Installed::Missing, Some(&written), VERSION),
                Plan::Keep(state.clone())
            );
        }
    }
    // Only a new grant asks again; a viewer's request asks unless declined.
    let declined = marker(SelfState::Declined, LoginTrigger::VersionStart);
    assert_eq!(
        decide(LoginTrigger::Grant, true, &old, Some(&declined), VERSION),
        Plan::Install
    );
    assert_eq!(
        decide(
            LoginTrigger::PeerRequest,
            true,
            &old,
            Some(&declined),
            VERSION
        ),
        Plan::SyncKeep(Some(SelfState::Declined))
    );
    // The watch never sets up, also not before the first result.
    assert_eq!(
        decide(LoginTrigger::Watch, true, &old, None, VERSION),
        Plan::SyncKeep(None)
    );
    // A current service is synced without any setup.
    let current = running(VERSION);
    assert_eq!(
        decide(LoginTrigger::VersionStart, true, &current, None, VERSION),
        Plan::Sync
    );
    // No steering device and no service: nothing at all, no prompt.
    assert_eq!(
        decide(
            LoginTrigger::VersionStart,
            false,
            &Installed::Missing,
            None,
            VERSION
        ),
        Plan::Nothing
    );
    assert_eq!(
        decide(
            LoginTrigger::Grant,
            true,
            &Installed::Unavailable,
            None,
            VERSION
        ),
        Plan::Keep(SelfState::Failed("unsupported".into()))
    );
}

#[test]
fn login_selbst_task_last_revocation_plans_removal() {
    // The service still runs; without a steering device it is synced first.
    assert_eq!(
        decide(
            LoginTrigger::CoverageEnded,
            false,
            &running(VERSION),
            None,
            VERSION
        ),
        Plan::Sync
    );
    assert!(removal_due(false, 0, 0, None));
    let done = marker(SelfState::Done, LoginTrigger::Grant);
    assert!(removal_due(false, 0, 0, Some(&done)));
    // Not while a device steers, a grant of this account remains or any
    // other (administrative or other account's) grant exists.
    assert!(!removal_due(true, 0, 0, None));
    assert!(!removal_due(false, 1, 0, None));
    assert!(!removal_due(false, 0, 1, None));
    // Once per version: removed or declined is not asked again.
    for state in [SelfState::Removed, SelfState::RemoveDeclined] {
        assert!(!removal_due(
            false,
            0,
            0,
            Some(&marker(state, LoginTrigger::CoverageEnded))
        ));
    }
    assert_eq!(state_for_exit(exit::DONE, true), SelfState::Removed);
    assert_eq!(
        state_for_exit(exit::UAC_DECLINED, true),
        SelfState::RemoveDeclined
    );
    assert_eq!(
        state_for_exit(exit::GRANTS_REMAIN, true),
        SelfState::Failed("grants_remain".into())
    );
    assert_eq!(Step::parse(STEP_REMOVE), Some(Step::Remove));
}

#[test]
fn login_selbst_task_revocation_reaches_an_older_running_service() {
    // A declined or failed update leaves the older service running with
    // its grants: every later grant change still syncs it, so a revoked
    // device loses the login grant there too (step 2 (f)).
    let old = running("0.5.196");
    for state in [
        SelfState::Declined,
        SelfState::Failed("hash_mismatch".into()),
    ] {
        let written = marker(state.clone(), LoginTrigger::Grant);
        assert_eq!(
            decide(
                LoginTrigger::CoverageEnded,
                true,
                &old,
                Some(&written),
                VERSION
            ),
            Plan::SyncKeep(Some(state.clone()))
        );
        assert_eq!(
            decide(
                LoginTrigger::ListChanged,
                true,
                &old,
                Some(&written),
                VERSION
            ),
            Plan::SyncKeep(Some(state))
        );
    }
    // The last revocation syncs the older service and then removes it.
    assert_eq!(
        decide(LoginTrigger::CoverageEnded, false, &old, None, VERSION),
        Plan::Sync
    );
}

#[test]
fn login_selbst_task_hash_mismatch_aborts_and_keeps_old_state() {
    let good = "ab".repeat(32);
    let other = "cd".repeat(32);
    assert!(image_binding(&good, &good, &good.to_uppercase()).is_ok());
    assert_eq!(image_binding(&good, &other, &good), Err("hash_mismatch"));
    assert_eq!(image_binding(&good, &good, &other), Err("hash_mismatch"));
    assert_eq!(
        image_binding("short", "short", "short"),
        Err("hash_mismatch")
    );
    assert_eq!(
        state_for_exit(exit::HASH_MISMATCH, false),
        SelfState::Failed("hash_mismatch".into())
    );
    assert!(state_for_exit(exit::HASH_MISMATCH, false)
        .text()
        .contains("der bisherige Stand bleibt"));
    // The approval names exactly one hash; anything else is no step.
    let step = Step::Install {
        sha256: good.clone(),
    };
    assert_eq!(Step::parse(&step.name()), Some(step));
    for bad in [
        "login-service:".to_string(),
        format!("login-service:{}", "zz".repeat(32)),
        format!("login-service:{good};rm"),
        "windows-rdp".to_string(),
    ] {
        assert_eq!(Step::parse(&bad), None);
    }
}

#[test]
fn login_selbst_task_install_keeps_identity_and_grants() {
    let identity = MachineIdentity::generate().expect("identity");
    let fresh = install_policy(None, &identity);
    assert_eq!(fresh.revision, 1);
    assert!(fresh.grants.is_empty());
    assert_eq!(fresh.machine_public_key, identity.public_key);
    let mut existing = fresh.clone();
    existing.revision = 41;
    existing.grants.push(crate::login::core::model::PeerGrant {
        peer_id: "admin-device".into(),
        public_key: iroh::SecretKey::from_bytes(&[3; 32]).public().to_string(),
        target_users: vec!["1000".into(), "1001".into()],
        not_before: 0,
        expires_at: 4_000_000_000,
        rights: crate::login::core::self_grants::ConsoleAbilities {
            before_login: true,
            locked: true,
            uac: false,
            sas: false,
            console: true,
        }
        .rights(),
    });
    let update = install_policy(Some(&existing), &identity);
    assert_eq!(update.revision, 42);
    assert_eq!(update.grants.len(), 1, "an administrative grant stays");
    assert_eq!(update.machine_id, identity.machine_id);
    for state in [
        SelfState::Done,
        SelfState::NotNeeded,
        SelfState::Declined,
        SelfState::Removed,
        SelfState::RemoveDeclined,
        SelfState::Failed("no_prompt".into()),
    ] {
        assert!(!state.text().is_empty());
    }
}

#[test]
fn login_selbst_task_script_failure_carries_installer_line() {
    // The installer's own last line reaches the status text.
    let line = "Installation ancestor permits non-root writes: /usr/libexec";
    let state = state_for_failure(exit::SCRIPT_FAILED, false, Some(line));
    assert_eq!(state, SelfState::Failed(format!("script_failed: {line}")));
    let text = state.text();
    assert!(text.starts_with("fehlgeschlagen: der Einrichtungsschritt des Betriebssystems"));
    assert!(
        text.ends_with(&format!("(Meldung des Schritts: {line})")),
        "{text}"
    );
    // Without a line the code's text stays alone; success never carries one.
    assert_eq!(
        state_for_failure(exit::SCRIPT_FAILED, false, None),
        SelfState::Failed("script_failed".into())
    );
    assert_eq!(
        state_for_failure(exit::DONE, false, Some(line)),
        SelfState::Done
    );
    assert_eq!(
        state_for_failure(exit::DONE, true, Some(line)),
        SelfState::Removed
    );
    // A `set -e` end names the command after the command's own line.
    let lines = vec![
        "Policy requires explicit peer".to_string(),
        format!("{COMMAND_FAILED} with code 1 (line 9): python3 -"),
    ];
    assert_eq!(
        installer_detail(&lines).as_deref(),
        Some(
            "Policy requires explicit peer (Installer command failed with code 1 (line 9): python3 -)"
        )
    );
    assert_eq!(
        installer_detail(&lines[..1]).as_deref(),
        Some("Policy requires explicit peer")
    );
    assert_eq!(installer_detail(&[]), None);
    // Control characters go, long lines end at a character boundary.
    assert_eq!(
        script_detail("  a\tb\u{1b}[0m  ").as_deref(),
        Some("a b [0m")
    );
    assert_eq!(script_detail(" \r "), None);
    let long = "ä".repeat(400);
    let cut = script_detail(&long).unwrap_or_default();
    assert!(cut.len() <= MAX_DETAIL && cut.chars().all(|c| c == 'ä'));
    // An unknown reason with a colon is shown as it is.
    assert_eq!(reason_text("Code 9: x"), "Code 9: x");
    assert_eq!(
        reason_text("se_missing: gone"),
        "se fehlt neben dem Programm (Meldung des Schritts: gone)"
    );
}

/// A worker without the app image beside it (terminal-only `se`) leaves
/// `image_missing`; the desktop worker of the same version still sets up.
#[test]
fn login_selbst_task_missing_image_does_not_block_the_version() {
    let failed = marker(
        SelfState::Failed("image_missing".into()),
        LoginTrigger::ListChanged,
    );
    for trigger in [LoginTrigger::VersionStart, LoginTrigger::ListChanged] {
        assert_eq!(
            decide(trigger, true, &Installed::Missing, Some(&failed), VERSION),
            Plan::Install
        );
    }
    assert_eq!(
        decide(
            LoginTrigger::Watch,
            true,
            &Installed::Missing,
            Some(&failed),
            VERSION
        ),
        Plan::Keep(SelfState::Failed("image_missing".into()))
    );
    // Any other failure of this version still waits for a grant.
    let other = marker(
        SelfState::Failed("script".into()),
        LoginTrigger::ListChanged,
    );
    assert_eq!(
        decide(
            LoginTrigger::VersionStart,
            true,
            &Installed::Missing,
            Some(&other),
            VERSION
        ),
        Plan::Keep(SelfState::Failed("script".into()))
    );
}
