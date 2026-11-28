//! Rights of the actors on the lists (contract point 1, plan S2) and the
//! write-only view of the agent.
use super::admin;
use super::doc::StoreDoc;
use super::model::{Actor, EntryUpdate, MaskError, NewEntry, Tolerance};

fn doc_with_list() -> (StoreDoc, super::model::ListId, super::model::EntryId) {
    let mut doc = StoreDoc::new();
    let list = admin::create_list(&mut doc, "Klienten").unwrap();
    let entry = admin::add_entries(
        &mut doc,
        Actor::Agent,
        list,
        vec![NewEntry::fuzzy("Mustermann", Tolerance::Low).with_label("Name")],
    )
    .unwrap()[0]
        .id;
    (doc, list, entry)
}

/// One change of the rights matrix by `actor`.
fn apply(change: &str, doc: &mut StoreDoc, actor: Actor) -> Result<(), MaskError> {
    let list = doc.lists[0].id;
    let entry = doc.lists[0].entries[0].id;
    let tolerance = |tolerance| EntryUpdate {
        tolerance: Some(tolerance),
        ..EntryUpdate::default()
    };
    match change {
        "add entry" => {
            admin::add_entries(doc, actor, list, vec![NewEntry::literal("Neu")]).map(|_| ())
        }
        "add scope" => admin::add_scope(doc, actor, list, "/srv/akten").map(|_| ()),
        "raise tolerance" => {
            admin::update_entry(doc, actor, list, entry, tolerance(Tolerance::High)).map(|_| ())
        }
        "lower tolerance" => {
            admin::update_entry(doc, actor, list, entry, tolerance(Tolerance::Off)).map(|_| ())
        }
        "change value" => {
            let update = EntryUpdate {
                value: Some("Musterfrau".into()),
                ..EntryUpdate::default()
            };
            admin::update_entry(doc, actor, list, entry, update).map(|_| ())
        }
        "remove entry" => admin::remove_entry(doc, actor, list, entry),
        "disable" => admin::set_enabled(doc, actor, list, false),
        "delete list" => admin::delete_list(doc, actor, list),
        other => panic!("unknown change {other}"),
    }
}

#[test]
fn masks_task_rights_matrix_per_actor() {
    use Actor::*;
    // Rows: change, then allowed for Agent, Cli, Gui, VerifiedUser on an
    // unlocked list and on a locked one.
    let strengthen = ([true, true, true, true], [false, true, true, true]);
    let weaken = ([false, false, false, true], [false, false, false, true]);
    let rows = [
        ("add entry", strengthen),
        ("add scope", strengthen),
        ("raise tolerance", strengthen),
        ("lower tolerance", weaken),
        ("change value", weaken),
        ("remove entry", weaken),
        ("disable", weaken),
        ("delete list", weaken),
    ];
    let actors = [Agent, Cli, Gui, VerifiedUser];
    for (name, (unlocked, locked)) in rows {
        for (lock, expected) in [(false, unlocked), (true, locked)] {
            for (actor, allowed) in actors.iter().zip(expected) {
                let (mut doc, list, _) = doc_with_list();
                admin::set_locked(&mut doc, Agent, list, lock).ok();
                if lock {
                    assert!(doc.lists[0].locked);
                }
                let before = doc.config_generation;
                let result = apply(name, &mut doc, *actor);
                assert_eq!(
                    result.is_ok(),
                    allowed,
                    "{name} by {actor:?}, locked {lock}"
                );
                if !allowed {
                    assert_eq!(doc.config_generation, before, "{name}: refused but changed");
                }
            }
        }
    }
}

#[test]
fn masks_task_unlock_needs_the_verified_user_and_locking_is_free() {
    let (mut doc, list, _) = doc_with_list();
    admin::set_locked(&mut doc, Actor::Agent, list, true).unwrap();
    for actor in [Actor::Agent, Actor::Cli, Actor::Gui] {
        assert!(
            admin::set_locked(&mut doc, actor, list, false).is_err(),
            "{actor:?}"
        );
    }
    admin::set_locked(&mut doc, Actor::VerifiedUser, list, false).unwrap();
    assert!(!doc.lists[0].locked);
}

#[test]
fn masks_task_summaries_never_carry_values_and_reveal_is_gui_only() {
    let (doc, list, _) = doc_with_list();
    let json = serde_json::to_string(&admin::summaries(&doc)).unwrap();
    assert!(!json.contains("Mustermann"), "{json}");
    for actor in [Actor::Agent, Actor::Cli, Actor::VerifiedUser] {
        assert!(admin::reveal(&doc, actor, list).is_err(), "{actor:?}");
    }
    let revealed = admin::reveal(&doc, Actor::Gui, list).unwrap();
    assert_eq!(revealed[0].value, "Mustermann");
}

#[test]
fn masks_task_invalid_entries_and_duplicates() {
    let (mut doc, list, entry) = doc_with_list();
    let bad = [
        NewEntry::regex("(unclosed"),
        NewEntry::literal(""),
        NewEntry::fuzzy("!!!", Tolerance::Low),
        NewEntry::literal("x").with_label("mit Leerzeichen"),
    ];
    for entry in bad {
        let result = admin::add_entries(&mut doc, Actor::Cli, list, vec![entry.clone()]);
        assert!(
            matches!(result, Err(MaskError::InvalidEntry(_))),
            "{entry:?}"
        );
    }
    let again = admin::add_entries(
        &mut doc,
        Actor::Cli,
        list,
        vec![NewEntry::fuzzy("Mustermann", Tolerance::Low).with_label("Name")],
    )
    .unwrap();
    assert_eq!(again[0].id, entry, "an equal entry is not added twice");
    assert_eq!(doc.lists[0].entries.len(), 1);
    assert!(matches!(
        admin::create_list(&mut doc, "KLIENTEN"),
        Err(MaskError::DuplicateName(_))
    ));
}
