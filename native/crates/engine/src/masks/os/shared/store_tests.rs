//! The sealed store: persistence across instances ("processes"), no clear
//! text on disk, lockdown state fail-closed, rollback refused, concurrent
//! assignment without duplicates, integrity guard, reset.
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use super::side;
use super::store::{MaskStore, SEALED_FILE, STATE_FILE};
use crate::masks::core::model::{Actor, LockdownState, MaskError, NewEntry, Tolerance};

const KEY: [u8; 32] = [7; 32];

fn open(dir: &Path) -> MaskStore {
    MaskStore::open_at(dir.join("masks"), KEY).unwrap()
}

fn file(dir: &Path, name: &str) -> std::path::PathBuf {
    dir.join("masks").join(name)
}

fn fresh() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    side::forget(&dir.path().join("masks"));
    dir
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}

#[test]
fn masks_task_store_persists_lists_and_placeholders_across_instances() {
    let dir = fresh();
    let first = open(dir.path());
    assert_eq!(first.lockdown_state(), LockdownState::Off);
    assert!(first.active().unwrap().is_empty());
    let list = first.create_list(Actor::Agent, "Klienten").unwrap();
    first
        .add_entries(
            Actor::Agent,
            list,
            vec![
                NewEntry::literal("Mustermann").with_label("Name"),
                NewEntry::fuzzy("Schweinsbraten", Tolerance::Medium),
            ],
        )
        .unwrap();
    let masked = first
        .active()
        .unwrap()
        .mask("Mustermann isst Weinbraten")
        .into_string();
    assert_eq!(masked, "⟦Name-1⟧ isst ⟦Maske-1.1⟧");
    // Nothing readable on disk.
    let sealed = std::fs::read(file(dir.path(), SEALED_FILE)).unwrap();
    let state = std::fs::read(file(dir.path(), STATE_FILE)).unwrap();
    for word in ["Mustermann", "Weinbraten", "Schweinsbraten", "Klienten"] {
        assert!(
            !contains(&sealed, word) && !contains(&state, word),
            "{word}"
        );
    }
    // A second instance (another process) resolves and reuses them.
    let second = open(dir.path());
    let set = second.active().unwrap();
    assert_eq!(set.unmask(&masked).unwrap(), "Mustermann isst Weinbraten");
    assert_eq!(
        set.mask("MUSTERMANN, Mustermann").as_str(),
        "⟦Name-1.1⟧, ⟦Name-1⟧"
    );
    // The first instance resolves the variant the second one assigned.
    assert_eq!(
        first.active().unwrap().unmask("⟦Name-1.1⟧").unwrap(),
        "MUSTERMANN"
    );
    // Summaries carry no value.
    let summary = serde_json::to_string(&second.lists().unwrap()).unwrap();
    assert!(!summary.contains("Mustermann"), "{summary}");
}

#[test]
fn masks_task_scope_switches_the_lockdown_and_state_holds_no_secret() {
    let dir = fresh();
    let store = open(dir.path());
    let list = store.create_list(Actor::Cli, "Akten").unwrap();
    assert_eq!(store.lockdown_state(), LockdownState::Off);
    store
        .add_scope(Actor::Cli, list, "/srv/akten/Mustermann")
        .unwrap();
    assert_eq!(store.lockdown_state(), LockdownState::On);
    let set = store.active().unwrap();
    assert!(set.lockdown());
    assert_eq!(
        set.local_scopes(),
        [std::path::PathBuf::from("/srv/akten/Mustermann")]
    );
    let state = std::fs::read_to_string(file(dir.path(), STATE_FILE)).unwrap();
    assert!(!state.contains("Mustermann"), "{state}");
    // An edited state.json alone does not lift the lockdown.
    let lifted = state.replace("\"lockdown\":true", "\"lockdown\":false");
    std::fs::write(file(dir.path(), STATE_FILE), lifted).unwrap();
    assert_eq!(store.lockdown_state(), LockdownState::On);
    // Only the verified user disables the list.
    assert!(store.set_enabled(Actor::Agent, list, false).is_err());
    store.set_enabled(Actor::VerifiedUser, list, false).unwrap();
    assert_eq!(store.lockdown_state(), LockdownState::Off);
}

#[test]
fn masks_task_store_fails_closed_when_files_are_missing_or_damaged() {
    let dir = fresh();
    let store = open(dir.path());
    let list = store.create_list(Actor::Cli, "Klienten").unwrap();
    store
        .add_entries(Actor::Cli, list, vec![NewEntry::literal("Mustermann")])
        .unwrap();
    let sealed = file(dir.path(), SEALED_FILE);
    let state = file(dir.path(), STATE_FILE);
    let good = std::fs::read(&sealed).unwrap();
    // state.json missing beside an existing store.
    std::fs::remove_file(&state).unwrap();
    assert_eq!(store.lockdown_state(), LockdownState::Unknown);
    // Both files missing: the side marker remembers the store.
    std::fs::remove_file(&sealed).unwrap();
    assert_eq!(store.lockdown_state(), LockdownState::Unknown);
    assert!(matches!(store.active(), Err(MaskError::Unreadable(_))));
    assert!(store.create_list(Actor::Cli, "Neu").is_err());
    // Damaged file.
    let mut damaged = good.clone();
    let last = damaged.len() - 1;
    damaged[last] ^= 0x55;
    std::fs::write(&sealed, damaged).unwrap();
    store.invalidate();
    assert!(matches!(store.active(), Err(MaskError::Unreadable(_))));
    // Another key.
    std::fs::write(&sealed, &good).unwrap();
    let other = MaskStore::open_at(dir.path().join("masks"), [9; 32]).unwrap();
    assert!(matches!(other.active(), Err(MaskError::Unreadable(_))));
    assert!(store.active().is_ok());
}

#[test]
fn masks_task_an_older_copy_of_the_store_is_refused() {
    let dir = fresh();
    let store = open(dir.path());
    let list = store.create_list(Actor::Cli, "Klienten").unwrap();
    let old = std::fs::read(file(dir.path(), SEALED_FILE)).unwrap();
    store
        .add_entries(Actor::Cli, list, vec![NewEntry::literal("Mustermann")])
        .unwrap();
    std::fs::write(file(dir.path(), SEALED_FILE), old).unwrap();
    store.invalidate();
    assert!(matches!(store.active(), Err(MaskError::Unreadable(_))));
}

#[test]
fn masks_task_concurrent_processes_never_share_a_placeholder() {
    let dir = fresh();
    let setup = open(dir.path());
    let list = setup.create_list(Actor::Cli, "Akten").unwrap();
    setup
        .add_entries(
            Actor::Cli,
            list,
            vec![NewEntry::regex(r"ID-\d+").with_label("Akte")],
        )
        .unwrap();
    let path = Arc::new(dir.path().to_path_buf());
    let workers: Vec<_> = (0..4)
        .map(|worker| {
            let path = Arc::clone(&path);
            std::thread::spawn(move || {
                let own = open(&path);
                let mut seen = Vec::new();
                for round in 0..25 {
                    // Overlapping values between workers and fresh ones.
                    let value = format!("ID-{}", (round * 3 + worker) % 40);
                    let masked = own.active().unwrap().mask(&value).into_string();
                    seen.push((value, masked));
                }
                seen
            })
        })
        .collect();
    let mut by_placeholder: BTreeMap<String, String> = BTreeMap::new();
    for worker in workers {
        for (value, masked) in worker.join().unwrap() {
            assert!(masked.starts_with("⟦Akte-"), "{masked}");
            let previous = by_placeholder.insert(masked.clone(), value.clone());
            assert!(
                previous.is_none_or(|p| p == value),
                "{masked} stands for two values"
            );
        }
    }
    let check = open(dir.path()).active().unwrap();
    for (masked, value) in &by_placeholder {
        assert_eq!(&check.unmask(masked).unwrap(), value);
    }
    let distinct: std::collections::BTreeSet<&String> = by_placeholder.values().collect();
    assert_eq!(
        distinct.len(),
        by_placeholder.len(),
        "one value, one placeholder"
    );
}

#[test]
fn masks_task_guard_restores_a_changed_store_and_adopts_se_changes() {
    let dir = fresh();
    let store = open(dir.path());
    let list = store.create_list(Actor::Cli, "Akten").unwrap();
    store.add_scope(Actor::Cli, list, "/srv/akten").unwrap();
    let mut guard = store.guard().unwrap();
    assert_eq!(guard.check_and_restore(), Ok(()));
    let sealed = file(dir.path(), SEALED_FILE);
    let state = file(dir.path(), STATE_FILE);
    let good = std::fs::read(&sealed).unwrap();
    // Deleted.
    std::fs::remove_file(&sealed).unwrap();
    std::fs::remove_file(&state).unwrap();
    assert!(guard.check_and_restore().is_err());
    assert_eq!(std::fs::read(&sealed).unwrap(), good);
    assert_eq!(store.lockdown_state(), LockdownState::On);
    assert_eq!(guard.check_and_restore(), Ok(()));
    // Replaced by garbage.
    std::fs::write(&sealed, b"SEMASK1\0garbage").unwrap();
    assert!(guard.check_and_restore().is_err());
    assert_eq!(std::fs::read(&sealed).unwrap(), good);
    // state.json edited.
    std::fs::write(
        &state,
        br#"{"format":1,"generation":0,"config_generation":0,"lockdown":false}"#,
    )
    .unwrap();
    assert!(guard.check_and_restore().is_err());
    assert!(std::fs::read_to_string(&state)
        .unwrap()
        .contains("\"lockdown\":true"));
    // A change through se (another instance) is adopted.
    open(dir.path())
        .add_entries(Actor::Cli, list, vec![NewEntry::literal("Neu")])
        .unwrap();
    assert_eq!(guard.check_and_restore(), Ok(()));
}

#[test]
fn masks_task_reset_needs_the_verified_user() {
    let dir = fresh();
    let store = open(dir.path());
    let list = store.create_list(Actor::Cli, "Akten").unwrap();
    store.add_scope(Actor::Cli, list, "/srv/akten").unwrap();
    for actor in [Actor::Agent, Actor::Cli, Actor::Gui] {
        assert!(store.reset(actor).is_err(), "{actor:?}");
    }
    store.reset(Actor::VerifiedUser).unwrap();
    assert_eq!(store.lockdown_state(), LockdownState::Off);
    assert!(store.active().unwrap().is_empty());
}
