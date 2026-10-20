//! Acceptance signals of VLT-M10 (U29) on the comparison window: the rows and
//! the buttons of the entries, the group filter, the collective actions and
//! the merge of a text conflict.
use std::collections::HashSet;

use super::vault_models_compare::{
    bulk_candidates, bulk_question, buttons_for, compare_rows, finding_title, item_label,
    merge_model, offers_text, open_sides, side_text, GroupFilter, RowModel,
};
use crate::daemon::{CompareItemWire, CompareSideWire};
use crate::vault::CompareGroup;

// ─── Comparison ───────────────────────────────────────────────────────────

fn side(exists: bool, size: Option<u64>, label: &str) -> CompareSideWire {
    CompareSideWire {
        exists,
        size,
        mtime_ms: None,
        location: label.into(),
    }
}

fn item(
    id: &str,
    group: &str,
    kind: &str,
    path: &str,
    sides: (bool, bool),
    actions: &[&str],
) -> CompareItemWire {
    CompareItemWire {
        item: id.into(),
        group: group.into(),
        kind: kind.into(),
        path: path.into(),
        a: side(sides.0, sides.0.then_some(2048), "Tresor A"),
        b: side(sides.1, sides.1.then_some(512), "Tresor B"),
        actions: actions.iter().map(|action| action.to_string()).collect(),
        finding: None,
        note: None,
    }
}

fn sample_items() -> Vec<CompareItemWire> {
    vec![
        item(
            "i1",
            "conflict",
            "file",
            "/Doku/a.txt",
            (true, true),
            &["keep-a", "keep-b", "keep-both", "merge"],
        ),
        item(
            "i2",
            "conflict",
            "file",
            "/Doku/b.txt",
            (true, false),
            &["keep-a", "keep-b"],
        ),
        item(
            "i3",
            "new_b",
            "file",
            "/neu.txt",
            (false, true),
            &["transfer"],
        ),
        item(
            "i4",
            "deleted_a",
            "file",
            "/weg.txt",
            (false, true),
            &["confirm-delete", "restore"],
        ),
        item(
            "i5",
            "structure",
            "node",
            "",
            (true, false),
            &["remove-node"],
        ),
    ]
}

#[test]
fn vault_task_gui_compare_rows_and_actions() {
    let items = sample_items();
    let done = HashSet::new();
    let list = compare_rows(&items, &GroupFilter::default(), &done);
    // Groups in the order of the plan, each with its header; the structure
    // entry is not listed by default.
    let headers: Vec<String> = list
        .iter()
        .filter_map(|row| match row {
            RowModel::Header { title, .. } => Some(title.clone()),
            RowModel::Item { .. } => None,
        })
        .collect();
    assert_eq!(
        headers,
        ["Konflikt (2)", "Neu auf B (1)", "Auf A gelöscht (1)"]
    );
    assert_eq!(list.len(), 3 + 4);
    // A row names the path and both sides; a missing side is „—“.
    let RowModel::Item {
        index,
        label,
        a,
        b,
        done: finished,
    } = &list[1]
    else {
        panic!("first entry row");
    };
    assert_eq!(
        (*index, label.as_str(), *finished),
        (0, "/Doku/a.txt", false)
    );
    assert_eq!(a, "A: 2.00 KB");
    assert_eq!(b, "B: 512 B");
    let RowModel::Item { b, .. } = &list[2] else {
        panic!("second entry row");
    };
    assert_eq!(b, "B: —");
    // An entry finished by an action stays in the list, marked.
    let finished: HashSet<String> = ["i3".to_string()].into();
    let marked = compare_rows(&items, &GroupFilter::default(), &finished);
    assert!(marked.iter().any(|row| matches!(
        row,
        RowModel::Item {
            index: 2,
            done: true,
            ..
        }
    )));
    // Size and time of a side, in the words of the interface.
    let mut stamped = side(true, Some(100), "A");
    stamped.mtime_ms = Some(1_700_000_000_000);
    let text = side_text(&stamped);
    let (size, time) = text.split_once(", ").expect("size, time");
    assert_eq!(size, "100 B");
    assert_eq!(time.len(), 16, "JJJJ-MM-TT HH:MM: {time}");
    assert_eq!(side_text(&side(false, None, "A")), "—");
    // The label of an entry without a path.
    let mut node = item("n1", "structure", "node", "", (true, false), &[]);
    node.note = Some("Knoten ohne Typdatei".into());
    assert_eq!(item_label(&node), "Knoten ohne Typdatei");
    node.finding = Some("UnknownType".into());
    node.note = None;
    assert_eq!(item_label(&node), "UnknownType");
    assert_eq!(finding_title(&node), "Knoten ohne Typdatei");
    // Collective actions exist only with a conflict of a file with two
    // sides, and ask first.
    assert_eq!(bulk_candidates(&items), 1);
    assert_eq!(bulk_candidates(&items[1..]), 0);
    assert_eq!(
        bulk_question(3, "Privat"),
        "Alle 3 Konflikte zugunsten von Privat auflösen?"
    );
    // The open buttons need a file on that side.
    assert_eq!(open_sides(&items[0]), [true, true]);
    assert_eq!(open_sides(&items[1]), [true, false]);
    assert_eq!(open_sides(&items[4]), [false, false]);
    // Only a conflict that offers `merge` shows the line view.
    assert!(offers_text(&items[0]));
    assert!(!offers_text(&items[1]));
}

#[test]
fn vault_task_gui_compare_actions_by_group() {
    let removed = "/daten/vault-removed";
    let titles = |item: &CompareItemWire| -> Vec<&'static str> {
        buttons_for(item, removed)
            .iter()
            .map(|button| button.title)
            .collect()
    };
    let make = |group: &str, kind: &str, sides: (bool, bool), actions: &[&str]| {
        item("x", group, kind, "/p/d.txt", sides, actions)
    };
    // U26-5: the buttons are the actions the worker offers, in its order.
    assert_eq!(
        titles(&make(
            "conflict",
            "file",
            (true, true),
            &["keep-a", "keep-b", "keep-both", "merge"]
        )),
        [
            "Fassung A behalten",
            "Fassung B behalten",
            "Beide behalten",
            "Zusammenführen"
        ]
    );
    assert_eq!(
        titles(&make(
            "conflict",
            "file",
            (true, false),
            &["keep-a", "keep-b"]
        )),
        ["Fassung A behalten", "Fassung B behalten"]
    );
    assert_eq!(
        titles(&make(
            "conflict",
            "folder",
            (true, true),
            &["merge-folders", "keep-folders-both"]
        )),
        ["Ordner zusammenführen", "Beide Ordner behalten"]
    );
    for group in ["changed_a", "changed_b", "new_a", "new_b"] {
        let buttons = buttons_for(&make(group, "file", (true, true), &["transfer"]), removed);
        assert_eq!(buttons.len(), 1, "{group}");
        assert_eq!(buttons[0].title, "Jetzt übertragen");
        assert_eq!(
            buttons[0].question, None,
            "{group}: a transfer asks nothing"
        );
    }
    for group in ["deleted_a", "deleted_b", "deferred"] {
        let buttons = buttons_for(
            &make(group, "file", (true, false), &["confirm-delete", "restore"]),
            removed,
        );
        let names: Vec<&str> = buttons.iter().map(|button| button.title).collect();
        assert_eq!(
            names,
            ["Löschung übernehmen", "Wiederherstellen"],
            "{group}"
        );
        let question = buttons[0].question.as_deref().expect("deleting asks");
        if group == "deferred" {
            assert!(question.contains("Verzeichnis-ID-Sicherung"), "{question}");
        } else {
            assert!(question.contains("/p/d.txt"), "{question}");
        }
        assert_eq!(buttons[1].question, None, "restoring asks nothing");
    }
    // The key file reads keep-a/keep-b as the choice of a version.
    assert_eq!(
        titles(&make(
            "conflict",
            "key",
            (true, true),
            &["keep-a", "keep-b"]
        )),
        ["Fassung A behalten", "Fassung B behalten"]
    );
    // Findings of the check: what removes cipher files asks and names the
    // folder that keeps a copy.
    for key in [
        "remove-node",
        "remove-duplicate",
        "remove-stage",
        "separate-folder",
    ] {
        let buttons = buttons_for(&make("structure", "node", (true, false), &[key]), removed);
        let question = buttons[0].question.as_deref().expect(key);
        assert!(question.contains(removed), "{key}: {question}");
    }
    let merge_folders = buttons_for(
        &make("conflict", "folder", (true, true), &["merge-folders"]),
        removed,
    );
    assert!(merge_folders[0]
        .question
        .as_deref()
        .expect("merging folders asks")
        .contains(removed));
    for key in [
        "create-content",
        "write-dirid",
        "attach-orphan",
        "rename-shortened",
    ] {
        let buttons = buttons_for(&make("structure", "node", (true, false), &[key]), removed);
        assert_eq!(buttons.len(), 1, "{key}");
        assert_eq!(buttons[0].question, None, "{key}");
    }
    // A name this build does not know has no button.
    assert!(buttons_for(
        &make("conflict", "file", (true, true), &["neu-in-v2"]),
        removed
    )
    .is_empty());
}

#[test]
fn vault_task_gui_group_filter_excludes_structure_by_default() {
    let items = sample_items();
    let mut filter = GroupFilter::default();
    assert!(!filter.is_on(CompareGroup::Structure));
    assert!(CompareGroup::DEFAULT
        .iter()
        .all(|group| filter.is_on(*group)));
    // The request asks for nothing special: the default of the worker.
    assert!(filter.requested().is_empty());
    assert!(!filter.shows_key("structure"));
    assert!(filter.shows_key("conflict"));
    let list = compare_rows(&items, &filter, &HashSet::new());
    assert!(!list
        .iter()
        .any(|row| matches!(row, RowModel::Item { index: 4, .. })));
    // With the structure on, the worker is asked for every group.
    filter.set(CompareGroup::Structure, true);
    let asked = filter.requested();
    assert_eq!(asked.len(), CompareGroup::ALL.len());
    assert!(asked.contains(&"structure") && asked.contains(&"conflict"));
    let list = compare_rows(&items, &filter, &HashSet::new());
    assert!(list.iter().any(|row| matches!(
        row,
        RowModel::Header {
            group: CompareGroup::Structure,
            ..
        }
    )));
    // A group switched off has no rows, whatever the worker delivered.
    filter.set(CompareGroup::Conflict, false);
    let list = compare_rows(&items, &filter, &HashSet::new());
    assert!(!list.iter().any(|row| matches!(
        row,
        RowModel::Header {
            group: CompareGroup::Conflict,
            ..
        }
    )));
}

#[test]
fn vault_task_gui_merge_model_follows_the_choices() {
    let mut model = merge_model("eins\nzwei\ndrei\n", "eins\nZWEI\ndrei\n").expect("model");
    assert!(model.differs());
    // The defaults keep side A; „Alle B“ takes the other version whole.
    assert_eq!(model.result(), "eins\nzwei\ndrei\n");
    model.take_all(false);
    assert_eq!(model.result(), "eins\nZWEI\ndrei\n");
    model.take_all(true);
    assert_eq!(model.result(), "eins\nzwei\ndrei\n");
    assert!(!merge_model("gleich\n", "gleich\n")
        .expect("model")
        .differs());
    // Mixed line ends cannot be merged (a copy of both stays instead).
    assert!(merge_model("a\r\nb\nc", "a\nb\nc").is_err());
}
