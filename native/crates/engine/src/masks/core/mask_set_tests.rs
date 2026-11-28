//! The masking set: placeholders, round trip, escaping, refusals, line
//! numbers, range map, unions, bytes and JSON spellings.
use std::sync::Arc;

use super::admin;
use super::doc::StoreDoc;
use super::mask_set::{MaskSet, MemoryAllocator};
use super::model::{Actor, NewEntry, Tolerance, UnmaskError};

pub(super) fn set_of(entries: Vec<NewEntry>) -> Arc<MaskSet> {
    let mut doc = StoreDoc::new();
    let list = admin::create_list(&mut doc, "Klienten").unwrap();
    admin::add_entries(&mut doc, Actor::Cli, list, entries).unwrap();
    let allocator = Arc::new(MemoryAllocator::new(doc.clone()));
    Arc::new(MaskSet::build(&doc, Some(allocator)).unwrap())
}

fn names() -> Arc<MaskSet> {
    set_of(vec![
        NewEntry::literal("Mustermann").with_label("Name"),
        NewEntry::regex(r"\b\d{3}-\d{4}\b").with_label("Akte"),
        NewEntry::fuzzy("Schweinsbraten", Tolerance::Medium).with_label("Speise"),
    ])
}

#[test]
fn masks_task_mask_replaces_values_and_unmask_restores_them() {
    let set = names();
    let text = "Herr Mustermann, Akte 123-4567 und 765-4321, isst Weinbraten. MUSTERMANN.";
    let masked = set.mask(text);
    assert_eq!(
        masked.as_str(),
        "Herr ⟦Name-1⟧, Akte ⟦Akte-1⟧ und ⟦Akte-2⟧, isst ⟦Speise-1.1⟧. ⟦Name-1.1⟧."
    );
    assert!(masked.is_complete());
    assert_eq!(set.unmask(masked.as_str()).unwrap(), text);
    // The same values keep their placeholders in later outputs.
    assert_eq!(
        set.mask("765-4321 Mustermann").as_str(),
        "⟦Akte-2⟧ ⟦Name-1⟧"
    );
    // ⟦X-N⟧ restores the list value, ⟦X-N.V⟧ the found form.
    assert_eq!(
        set.unmask("⟦Speise-1⟧/⟦Speise-1.1⟧").unwrap(),
        "Schweinsbraten/Weinbraten"
    );
}

#[test]
fn masks_task_original_brackets_are_escaped_and_restored() {
    let set = names();
    let text = "Form ⟦Name-1⟧ steht so im Text, ⟦⟦ auch, ⟦Mustermann";
    let masked = set.mask(text);
    assert_eq!(
        masked.as_str(),
        "Form ⟦⟦Name-1⟧ steht so im Text, ⟦⟦⟦⟦ auch, ⟦⟦⟦Name-1⟧"
    );
    assert_eq!(set.unmask(masked.as_str()).unwrap(), text);
}

#[test]
fn masks_task_unknown_and_malformed_placeholders_are_refused() {
    let set = names();
    assert_eq!(
        set.unmask("schreibe ⟦Name-99⟧"),
        Err(UnmaskError::Unknown("⟦Name-99⟧".into()))
    );
    assert_eq!(
        set.unmask("⟦Name-0⟧"),
        Err(UnmaskError::Unknown("⟦Name-0⟧".into()))
    );
    assert_eq!(
        set.unmask("ab ⟦ cd"),
        Err(UnmaskError::Malformed { offset: 3 })
    );
    assert_eq!(set.unmask("ohne Platzhalter").unwrap(), "ohne Platzhalter");
}

#[test]
fn masks_task_multi_line_matches_keep_the_line_count() {
    let set = set_of(vec![
        NewEntry::regex(r"Musterweg \d+\r?\n\d{5} \w+").with_label("Adresse")
    ]);
    let text = "Zeile 1\nMusterweg 5\r\n12345 Stadt\nZeile 4\n";
    let masked = set.mask(text);
    assert_eq!(masked.as_str(), "Zeile 1\n⟦Adresse-1\r\n⟧\nZeile 4\n");
    assert_eq!(masked.as_str().lines().count(), text.lines().count());
    assert_eq!(set.unmask(masked.as_str()).unwrap(), text);
    // Without the inner line ending the placeholder still resolves.
    assert_eq!(
        set.unmask("⟦Adresse-1⟧").unwrap(),
        "Musterweg 5\r\n12345 Stadt"
    );
}

#[test]
fn masks_task_map_range_follows_placeholders_and_escapes() {
    let set = names();
    let text = "ab Mustermann cd ⟦ ef";
    let masked = set.mask(text);
    assert_eq!(masked.as_str(), "ab ⟦Name-1⟧ cd ⟦⟦ ef");
    let value = 3..13;
    assert_eq!(&masked.as_str()[masked.map_range(value)], "⟦Name-1⟧");
    // Part of a value maps onto the whole placeholder.
    assert_eq!(&masked.as_str()[masked.map_range(5..7)], "⟦Name-1⟧");
    let after = text.find("cd").unwrap();
    assert_eq!(&masked.as_str()[masked.map_range(after..after + 2)], "cd");
    let ef = text.find("ef").unwrap();
    assert_eq!(&masked.as_str()[masked.map_range(ef..ef + 2)], "ef");
}

#[test]
fn masks_task_overlapping_hits_merge_into_one_placeholder() {
    let set = set_of(vec![
        NewEntry::literal("Max Muster").with_label("Name"),
        NewEntry::literal("Muster GmbH").with_label("Firma"),
    ]);
    let text = "bei Max Muster GmbH";
    let masked = set.mask(text);
    // The union stays one placeholder; no part of either value shows.
    assert_eq!(masked.as_str(), "bei ⟦Firma-1.1⟧");
    assert_eq!(set.unmask(masked.as_str()).unwrap(), text);
}

#[test]
fn masks_task_literal_options_case_and_whole_word() {
    let set = set_of(vec![{
        let mut entry = NewEntry::literal("Ost").with_label("Ort");
        entry.case_sensitive = true;
        entry.whole_word = true;
        entry
    }]);
    assert_eq!(set.mask("Ost, Osten, ost").as_str(), "⟦Ort-1⟧, Osten, ost");
}

#[test]
fn masks_task_empty_set_leaves_every_text_unchanged() {
    let set = MaskSet::empty();
    let text = "⟦ kein Platzhalter ⟦Name-1⟧";
    assert!(set.is_empty());
    assert_eq!(set.mask(text).as_str(), text);
    assert!(!set.mask(text).is_changed());
    assert_eq!(set.unmask(text).unwrap(), text);
    assert_eq!(set.mask_bytes(b"\xff\x00"), b"\xff\x00");
}

#[test]
fn masks_task_bytes_mask_literals_regex_and_fuzzy_in_utf8_sections() {
    let set = names();
    let bytes = b"\xff\xfeMustermann\x00Weinbraten\x80 123-4567";
    let masked = set.mask_bytes(bytes);
    let expected: Vec<u8> = [
        &b"\xff\xfe"[..],
        "⟦Name-1⟧".as_bytes(),
        &b"\x00"[..],
        "⟦Speise-1.1⟧".as_bytes(),
        &b"\x80 "[..],
        "⟦Akte-1⟧".as_bytes(),
    ]
    .concat();
    assert_eq!(masked, expected);
    assert_eq!(set.unmask_bytes(&masked).unwrap(), bytes);
}

#[test]
fn masks_task_paths_are_masked_as_a_whole() {
    let set = set_of(vec![
        NewEntry::literal("/srv/akten/Mustermann").with_label("Pfad")
    ]);
    assert_eq!(
        set.mask_path("/srv/akten/Mustermann/brief.txt"),
        "⟦Pfad-1⟧/brief.txt"
    );
    assert_eq!(
        set.unmask("⟦Pfad-1⟧/neu.txt").unwrap(),
        "/srv/akten/Mustermann/neu.txt"
    );
}

#[test]
fn masks_task_json_escaped_spellings_are_masked_and_restored_unchanged() {
    let set = set_of(vec![
        NewEntry::literal(r"C:\Users\Mustermann").with_label("Pfad"),
        NewEntry::fuzzy("Max \"Mäxchen\" Muster", Tolerance::Low).with_label("Name"),
    ]);
    let json = r#"{"path":"C:\\Users\\Mustermann\\x","nested":"{\"p\":\"C:\\\\Users\\\\Mustermann\"}","n":"Max \"Mäxchen\" Muster"}"#;
    let masked = set.mask(json);
    assert!(
        !masked.as_str().contains("Mustermann"),
        "{}",
        masked.as_str()
    );
    assert!(!masked.as_str().contains("Mäxchen"), "{}", masked.as_str());
    assert_eq!(set.unmask(masked.as_str()).unwrap(), json);
    // Plain text keeps the plain placeholder.
    assert_eq!(set.mask(r"C:\Users\Mustermann").as_str(), "⟦Pfad-1⟧");
}

#[test]
fn masks_task_round_trip_holds_for_mixed_text() {
    let set = names();
    let samples = [
        "",
        "⟦",
        "⟦⟦⟦",
        "Mustermann⟦Mustermann⟧Mustermann",
        "Schweins-\nbraten und Schwein Braten\r\n123-4567\n",
        "ä ö ü ß ⟦Akte-1⟧ Weinsbraten",
    ];
    for sample in samples {
        let masked = set.mask(sample);
        assert_eq!(set.unmask(masked.as_str()).unwrap(), sample, "{sample:?}");
        assert!(!masked.as_str().contains("Mustermann"), "{sample:?}");
    }
}
