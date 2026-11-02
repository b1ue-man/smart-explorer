//! Package G2 of plan 2026-10-31 §F: the state logic of the compact rights
//! window. A preset fills the switches, „Bestätigen“ hands exactly the
//! switches to the core, „Abbrechen“ calls cancel, and a connection that
//! waits for its rights opens the window. The core is a recorder here; the
//! real transactions are covered by `abgleich_task_connection_rights_tests`.
use super::*;
use crate::share::access_grant::{AccountProfile, Capability, CapabilityDecision};
use crate::share::connection_rights::{
    ConnectionRights, FolderChoice, RightsOrigin, RightsPreset, RightsTarget,
};
use crate::share::connection_rights_actions::RightsChange;
use crate::share::connection_rights_apply::PendingRightsView;
use crate::share::ShareProfiles;
use std::time::{Duration, Instant};

/// Folders chosen per path: (export name, path).
type Folders = Vec<(String, String)>;

#[derive(Default)]
struct Recorder {
    confirms: Vec<(RightsTarget, ConnectionRights, Folders)>,
    cancels: Vec<RightsTarget>,
    codes: Vec<(Option<ConnectionRights>, Folders)>,
}

fn change() -> RightsChange {
    RightsChange {
        profiles: ShareProfiles::default(),
        changed: true,
    }
}

impl RightsBackend for Recorder {
    fn confirm(
        &mut self,
        target: &RightsTarget,
        rights: &ConnectionRights,
        added: &[(String, String)],
    ) -> Result<RightsChange, String> {
        self.confirms
            .push((target.clone(), rights.clone(), added.to_vec()));
        Ok(change())
    }

    fn cancel(&mut self, target: &RightsTarget) -> Result<RightsChange, String> {
        self.cancels.push(target.clone());
        Ok(change())
    }

    fn set_code_rights(
        &mut self,
        rights: Option<&ConnectionRights>,
        added: &[(String, String)],
    ) -> Result<RightsChange, String> {
        self.codes.push((rights.cloned(), added.to_vec()));
        Ok(change())
    }
}

fn target(device: &str) -> RightsTarget {
    RightsTarget::Direct {
        device_id: device.into(),
        public_key: format!("key-{device}"),
    }
}

fn view(device: &str, origin: RightsOrigin) -> PendingRightsView {
    PendingRightsView {
        target: target(device),
        origin,
        since: 10,
        name: format!("Gerät {device}"),
        rights: ConnectionRights::none(),
    }
}

fn pending_draft(device: &str) -> RightsDraft {
    RightsDraft::for_pending(&view(device, RightsOrigin::Request))
}

#[test]
fn abgleich_task_preset_fills_every_switch_and_keeps_account_and_duration() {
    for preset in RightsPreset::ALL {
        let mut draft = pending_draft("a");
        draft.apply_preset(preset);
        assert_eq!(draft.rights, preset.rights(), "{preset:?}");
        assert_eq!(draft.selected_preset(), Some(preset));
    }
    let mut draft = pending_draft("a");
    draft.set_account(AccountProfile::Remote);
    draft.rights.valid_until = Some(5_000);
    draft.apply_preset(RightsPreset::FilesExec);
    assert_eq!(draft.rights.account, AccountProfile::Remote);
    assert_eq!(draft.rights.valid_until, Some(5_000));
    assert_eq!(draft.rights.commands, CapabilityDecision::Allow);
    assert_eq!(draft.rights.terminal, CapabilityDecision::Allow);
    assert_eq!(draft.rights.screen_view, CapabilityDecision::Off);
    assert!(draft.rights.write);
}

#[test]
fn abgleich_task_every_switch_stays_changeable_after_a_preset() {
    let mut draft = pending_draft("a");
    draft.apply_preset(RightsPreset::All);
    draft.set_decision(Capability::ScreenControl, CapabilityDecision::Ask);
    draft.rights.write = false;
    assert_eq!(draft.selected_preset(), None, "no longer exactly a preset");
    assert_eq!(draft.rights.screen_view, CapabilityDecision::Allow);
    assert_eq!(draft.rights.screen_control, CapabilityDecision::Ask);
    assert_eq!(draft.rights.commands, CapabilityDecision::Allow);
    assert!(!draft.rights.write);
    draft.apply_preset(RightsPreset::None);
    assert_eq!(draft.selected_preset(), Some(RightsPreset::None));
}

#[test]
fn abgleich_task_confirm_calls_the_core_with_exactly_the_chosen_rights() {
    let mut draft = pending_draft("a");
    draft.apply_preset(RightsPreset::All);
    draft.set_decision(Capability::ScreenView, CapabilityDecision::Ask);
    draft.expiry = ExpiryChoice::Day;
    let mut backend = Recorder::default();
    draft.confirm(&mut backend, 1_000).expect("confirmed");
    assert!(backend.cancels.is_empty() && backend.codes.is_empty());
    let [(sent_target, sent, added)] = backend.confirms.as_slice() else {
        panic!("one confirm expected: {}", backend.confirms.len());
    };
    assert_eq!(sent_target, &target("a"));
    let expected = ConnectionRights {
        screen_view: CapabilityDecision::Ask,
        valid_until: Some(1_000 + 86_400),
        ..RightsPreset::All.rights()
    };
    assert_eq!(sent, &expected);
    assert!(added.is_empty());
}

#[test]
fn abgleich_task_confirm_without_changes_sets_no_rights_instead_of_guessing() {
    let draft = pending_draft("a");
    let mut backend = Recorder::default();
    draft.confirm(&mut backend, 1).expect("confirmed");
    assert_eq!(backend.confirms[0].1, ConnectionRights::none());
}

#[test]
fn abgleich_task_confirm_of_a_room_and_of_the_code_use_their_own_core_call() {
    let room = RightsTarget::Room {
        profile_id: "room-1".into(),
    };
    let mut draft = RightsDraft::for_pending(&PendingRightsView {
        target: room.clone(),
        origin: RightsOrigin::RoomCreated,
        since: 1,
        name: "Büro".into(),
        rights: ConnectionRights::none(),
    });
    draft.apply_preset(RightsPreset::ReadWrite);
    let mut backend = Recorder::default();
    draft.confirm(&mut backend, 1).expect("room confirmed");
    assert_eq!(backend.confirms[0].0, room);
    assert_eq!(backend.confirms[0].1, RightsPreset::ReadWrite.rights());

    let mut code = RightsDraft::for_code(None);
    code.apply_preset(RightsPreset::ReadOnly);
    code.confirm(&mut backend, 1).expect("code confirmed");
    assert_eq!(backend.confirms.len(), 1, "the code is not a connection");
    assert_eq!(backend.codes.len(), 1);
    assert_eq!(backend.codes[0].0, Some(RightsPreset::ReadOnly.rights()));
}

#[test]
fn abgleich_task_cancel_calls_cancel_for_a_waiting_connection_only() {
    let mut backend = Recorder::default();
    let waiting = pending_draft("a");
    assert!(waiting.cancel(&mut backend).expect("cancelled").is_some());
    assert_eq!(backend.cancels, vec![target("a")]);
    assert!(backend.confirms.is_empty(), "cancel never sets rights");

    let manual = RightsDraft::new(
        RightsSubject::Connection(target("b")),
        "Gerät b",
        RightsPreset::ReadOnly.rights(),
    );
    assert!(manual.cancel(&mut backend).expect("closed").is_none());
    assert!(RightsDraft::for_code(None)
        .cancel(&mut backend)
        .expect("closed")
        .is_none());
    assert_eq!(
        backend.cancels.len(),
        1,
        "only the waiting one was cancelled"
    );
}

#[test]
fn abgleich_task_invalid_switches_are_refused_before_the_core_is_called() {
    let mut draft = pending_draft("a");
    draft.apply_preset(RightsPreset::FilesExec);
    draft.set_account(AccountProfile::Account {
        name: String::new(),
    });
    let mut backend = Recorder::default();
    assert!(draft.confirm(&mut backend, 1).is_err());
    assert!(backend.confirms.is_empty());
}

#[test]
fn abgleich_task_waiting_connections_open_the_window_one_after_the_other() {
    let now = Instant::now();
    let mut state = RightsWindowState::default();
    state.sync(&[], now);
    assert!(state.draft.is_none(), "nothing waits, nothing opens");

    let a = view("a", RightsOrigin::Request);
    let b = view("b", RightsOrigin::Pairing);
    state.sync(&[a.clone(), b.clone()], now);
    let draft = state.draft.as_ref().expect("opened for the first");
    assert_eq!(draft.name, "Gerät a");
    assert_eq!(draft.origin, Some(RightsOrigin::Request));
    assert_eq!(draft.subject, RightsSubject::Connection(target("a")));

    // Edits survive the next frame.
    state
        .draft
        .as_mut()
        .unwrap()
        .apply_preset(RightsPreset::ReadOnly);
    state.sync(&[a.clone(), b.clone()], now);
    assert_eq!(
        state.draft.as_ref().unwrap().selected_preset(),
        Some(RightsPreset::ReadOnly)
    );

    // Close button: put off, the next one opens, the first does not return.
    state.later();
    state.sync(&[a.clone(), b.clone()], now);
    assert_eq!(state.draft.as_ref().unwrap().name, "Gerät b");
    state.later();
    state.sync(&[a.clone(), b], now);
    assert!(state.draft.is_none());

    // Once settled elsewhere and waiting again, it opens again.
    state.sync(&[], now);
    assert!(state.dismissed.is_empty());
    state.sync(&[a], now);
    assert_eq!(state.draft.as_ref().unwrap().name, "Gerät a");
}

#[test]
fn abgleich_task_a_draft_of_a_settled_confirmation_closes_and_a_stale_snapshot_does_not_reopen() {
    let now = Instant::now();
    let mut state = RightsWindowState::default();
    let a = view("a", RightsOrigin::Request);
    state.sync(std::slice::from_ref(&a), now);
    assert!(state.draft.is_some());
    // Settled by the CLI: the next frame no longer lists it.
    state.sync(&[], now);
    assert!(state.draft.is_none());

    // Confirmed here: an older worker snapshot still lists it for a moment.
    state.sync(std::slice::from_ref(&a), now);
    let draft = state.draft.clone().expect("open");
    state.settled(&draft, now);
    assert!(state.draft.is_none());
    state.sync(std::slice::from_ref(&a), now + Duration::from_secs(1));
    assert!(state.draft.is_none(), "stale snapshot ignored");
    state.sync(&[a], now + Duration::from_secs(30));
    assert!(state.draft.is_some(), "a real new wait opens it again");
}

#[test]
fn abgleich_task_opened_by_hand_the_window_stays_until_closed() {
    let now = Instant::now();
    let mut state = RightsWindowState::default();
    state.open(RightsDraft::new(
        RightsSubject::Connection(target("a")),
        "Gerät a",
        RightsPreset::ReadOnly.rights(),
    ));
    state.sync(&[], now);
    assert!(state.draft.is_some(), "not waiting, opened on purpose");
    state.later();
    assert!(state.draft.is_none());
    assert!(state.dismissed.is_empty(), "only waiting ones are put off");
}

#[test]
fn abgleich_task_folder_selection_starts_full_and_an_empty_one_means_none() {
    let available = vec!["/a".to_string(), "/b".to_string()];
    let mut draft = pending_draft("a");
    draft.set_folder_mode(FolderMode::Selected, &available);
    assert!(draft.folder_ticked("/a") && draft.folder_ticked("/b"));
    draft.tick_folder("/b", false);
    assert_eq!(
        draft.finish(1).unwrap().folders,
        FolderChoice::Selected {
            paths: vec!["/a".into()],
            connections: false
        }
    );
    draft.tick_folder("/a", false);
    assert_eq!(draft.finish(1).unwrap().folders, FolderChoice::None);
    draft.set_folder_mode(FolderMode::All, &available);
    assert_eq!(draft.finish(1).unwrap().folders, FolderChoice::All);
}

#[test]
fn abgleich_task_added_folders_are_selected_and_exported_with_the_confirmation() {
    let available = vec!["/a".to_string()];
    let mut draft = pending_draft("a");
    draft.add_folder("Fotos", "C:\\Fotos", &available);
    draft.add_folder("Fotos", "C:/Fotos", &available);
    assert_eq!(
        draft.added_folders,
        vec![("Fotos".into(), "C:/Fotos".into())]
    );
    assert_eq!(draft.folder_mode(), FolderMode::Selected);
    assert!(draft.folder_ticked("C:/Fotos"));
    let mut backend = Recorder::default();
    draft.confirm(&mut backend, 1).expect("confirmed");
    assert_eq!(backend.confirms[0].2, draft.added_folders);
    // A cancelled draft hands nothing to the core: no export is created.
    let cancelled = pending_draft("b");
    cancelled.cancel(&mut backend).expect("cancelled");
    assert_eq!(backend.confirms.len(), 1);
}

#[test]
fn abgleich_task_expiry_counts_from_the_host_clock_and_keeps_an_existing_end() {
    assert_eq!(ExpiryChoice::Unlimited.resolve(100), None);
    assert_eq!(ExpiryChoice::Hour.resolve(100), Some(3_700));
    assert_eq!(ExpiryChoice::Week.resolve(0), Some(604_800));
    assert_eq!(ExpiryChoice::of(Some(9)), ExpiryChoice::Keep(9));
    assert_eq!(ExpiryChoice::Keep(9).resolve(100), Some(9));
    let mut rights = ConnectionRights::none();
    rights.valid_until = Some(77);
    let draft = RightsDraft::new(RightsSubject::Code, "c", rights);
    assert_eq!(draft.finish(1_000).unwrap().valid_until, Some(77));
}

#[test]
fn abgleich_task_auto_accept_text_describes_what_acceptance_really_gives() {
    let none = auto_accept_text(None);
    assert!(none.contains("keine Rechte"), "{none}");
    assert!(none.contains("ohne deine Bestätigung"), "{none}");
    let rightless = auto_accept_text(Some(&RightsPreset::None.rights()));
    assert_eq!(rightless, none);
    let files = auto_accept_text(Some(&RightsPreset::ReadWrite.rights()));
    assert!(files.contains("erhalten sofort"), "{files}");
    assert!(files.contains(RightsPreset::ReadWrite.label()), "{files}");
    assert!(!files.contains("keine Rechte"), "{files}");
    assert!(code_rights_text(None).contains("erfragt"));
    assert!(code_rights_text(Some(&RightsPreset::All.rights())).contains("Alles"));
}
