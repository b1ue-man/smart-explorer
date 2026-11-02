//! Exec grants in the stored profile: what 0.5.180 wrote reads unchanged, and
//! what narrowed grants write stays switched off for 0.5.180.
use serde::Deserialize;

use super::super::encode_profiles;
use super::{direct_grant, room_with_member, FakePersistence};
use super::{ShareProfiles, SHARE_PROFILE_VERSION};
use crate::share::{ExecGrant, ExecRunAs};

/// The five fields of `ExecGrant` in 0.5.180, as that version reads them.
#[derive(Debug, Deserialize)]
struct HeadExec {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    policy_revision: u64,
}

fn enabled(revision: u64) -> ExecGrant {
    ExecGrant {
        enabled: true,
        policy_revision: revision,
        changed_at: 7,
        ..ExecGrant::default()
    }
}

fn profiles_with(direct: ExecGrant, member: ExecGrant) -> ShareProfiles {
    let mut profiles = ShareProfiles::default();
    profiles.direct_grants.push(direct_grant(direct));
    profiles.rooms.push(room_with_member(member));
    profiles
}

#[test]
fn luecken_task_a_profile_of_0_5_180_loads_as_standard_user_and_saves_unchanged() {
    let mut value = serde_json::to_value(profiles_with(enabled(7), enabled(3))).unwrap();
    assert_eq!(value["schema_version"], SHARE_PROFILE_VERSION);
    // Exactly the fields 0.5.180 wrote: no run_as, no programs.
    for pointer in ["/direct_grants/0/exec", "/rooms/0/members/0/exec"] {
        let fields = value.pointer_mut(pointer).unwrap().as_object_mut().unwrap();
        fields.remove("run_as");
        fields.remove("programs");
        assert_eq!(fields.len(), 5);
    }
    let mut storage = FakePersistence {
        profiles: Some(serde_json::to_string_pretty(&value).unwrap()),
        ..FakePersistence::default()
    };
    let mut loaded = ShareProfiles::load_checked_with(None, &mut storage).unwrap();
    for grant in [
        &loaded.direct_grants[0].exec,
        &loaded.rooms[0].members[0].exec,
    ] {
        assert!(grant.enabled, "an existing grant stays enabled");
        assert_eq!(grant.run_as, ExecRunAs::Standard);
        assert!(grant.programs.is_empty(), "and keeps the full shell");
    }
    assert_eq!(loaded.direct_grants[0].exec.policy_revision, 7);
    assert_eq!(loaded.rooms[0].members[0].exec.policy_revision, 3);

    // The first save changes neither the switch nor the revision.
    loaded.save_with(&mut storage).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(storage.profiles.as_deref().unwrap()).unwrap();
    for (path, revision) in [
        (&saved["direct_grants"][0]["exec"], 7),
        (&saved["rooms"][0]["members"][0]["exec"], 3),
    ] {
        let head: HeadExec = serde_json::from_value(path.clone()).unwrap();
        assert!(head.enabled);
        assert_eq!(head.policy_revision, revision);
    }
    let again = ShareProfiles::load_checked_with(None, &mut storage).unwrap();
    assert_eq!(again.direct_grants[0].exec, loaded.direct_grants[0].exec);
    assert_eq!(
        again.rooms[0].members[0].exec,
        loaded.rooms[0].members[0].exec
    );
}

#[test]
fn luecken_task_narrowed_direct_and_room_grants_are_stored_switched_off_for_0_5_180() {
    let mut programs = enabled(5);
    programs.programs = vec!["/usr/bin/uptime".into()];
    let mut account = enabled(6);
    account.run_as = ExecRunAs::Account {
        name: "deploy".into(),
    };
    let profiles = profiles_with(programs.clone(), account.clone());
    let text = encode_profiles(&profiles).unwrap();
    let stored: serde_json::Value = serde_json::from_str(&text).unwrap();

    for (path, revision) in [
        (&stored["direct_grants"][0]["exec"], 5),
        (&stored["rooms"][0]["members"][0]["exec"], 6),
    ] {
        let head: HeadExec = serde_json::from_value(path.clone()).unwrap();
        assert!(!head.enabled, "0.5.180 must read a narrowed grant as off");
        assert_eq!(head.policy_revision, revision);
        assert_eq!(path["restricted_enabled"], true);
    }

    // This version reads both grants unchanged, and keeps writing them so.
    let mut storage = FakePersistence {
        profiles: Some(text),
        ..FakePersistence::default()
    };
    let mut loaded = ShareProfiles::load_checked_with(None, &mut storage).unwrap();
    assert_eq!(loaded.direct_grants[0].exec, programs);
    assert_eq!(loaded.rooms[0].members[0].exec, account);
    loaded.save_with(&mut storage).unwrap();
    let resaved: serde_json::Value =
        serde_json::from_str(storage.profiles.as_deref().unwrap()).unwrap();
    assert_eq!(resaved["direct_grants"][0]["exec"]["enabled"], false);
    assert_eq!(resaved["rooms"][0]["members"][0]["exec"]["enabled"], false);
    let again = ShareProfiles::load_checked_with(None, &mut storage).unwrap();
    assert!(again.direct_grants[0].exec.enabled && again.rooms[0].members[0].exec.enabled);
}
