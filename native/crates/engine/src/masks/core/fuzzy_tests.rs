//! The fuzzy rule against the 20 recalculated cases of reference
//! docs/refs/maskierung-fuzzy-ersetzen-2026-11-28.md §2.4 and the
//! pitfalls of §2.2 (borrowed neighbour letters) and plan S2 (compounds).
use super::doc::EntryDoc;
use super::fuzzy::FuzzyPattern;
use super::matcher::Matcher;
use super::model::{EntryId, EntryKind, ListId, Tolerance};

fn entry(value: &str, tolerance: Tolerance) -> EntryDoc {
    EntryDoc {
        id: EntryId(1),
        kind: EntryKind::Fuzzy,
        value: value.into(),
        label: "Speise".into(),
        case_sensitive: false,
        whole_word: false,
        tolerance,
    }
}

/// The masked substrings of `text`.
fn hits(value: &str, tolerance: Tolerance, text: &str) -> Vec<String> {
    let entry = entry(value, tolerance);
    let matcher = Matcher::compile([(ListId(1), &entry)]).unwrap();
    matcher
        .find(text.as_bytes())
        .into_iter()
        .map(|span| text[span.start..span.end].to_string())
        .collect()
}

/// Reference §2.4 table, column „soll“ (and the plan's „ja“ for
/// „Schweinebraten“), with `mittel`: trim ⌊0.3·14⌋ = 4, inner AUTO.
const CASES: &[(&str, bool)] = &[
    ("Schweinsbraten", true),
    ("Schwein Braten", true),
    ("Schweins-\nbraten", true),
    ("Weinsbraten", true),
    ("Weinbraten", true),
    ("Schweinsbratcn", true),
    ("Scbweinsbraten", true),
    ("Schwcinsbratcn", true),
    ("Schweinsbarten", true),
    ("Schweinebraten", true),
    ("Weinbrand", false),
    ("Schweinshaxe", false),
    ("Schweinsohren", false),
    ("Rinderbraten", false),
    ("Braten", false),
    ("Schwein", false),
    ("Weinberg", false),
    ("Sx#q!nzvr4tqm", false),
    ("ll1|;:.,'iI!", false),
    ("Scwnbrtxq", false),
];

#[test]
fn masks_task_fuzzy_medium_separates_the_twenty_reference_cases() {
    for (candidate, masked) in CASES {
        let found = hits("Schweinsbraten", Tolerance::Medium, candidate);
        let expected: Vec<String> = if *masked {
            vec![candidate.to_string()]
        } else {
            Vec::new()
        };
        assert_eq!(found, expected, "{candidate:?}");
    }
}

#[test]
fn masks_task_fuzzy_levels_widen_step_by_step() {
    // Rows: candidate, then masked at off, low, medium, high.
    let rows: &[(&str, [bool; 4])] = &[
        ("Schweinsbraten", [true, true, true, true]),
        ("SCHWEINS-BRATEN", [true, true, true, true]),
        ("Schwein Braten", [false, true, true, true]),
        ("Schweinsbratcn", [false, true, true, true]),
        ("Weinsbraten", [false, false, true, true]),
        ("Weinbraten", [false, false, true, true]),
        ("Schweinsohren", [false, false, false, false]),
        ("Rinderbraten", [false, false, false, false]),
        ("Scwnbrtxq", [false, false, false, false]),
    ];
    let levels = [
        Tolerance::Off,
        Tolerance::Low,
        Tolerance::Medium,
        Tolerance::High,
    ];
    for (candidate, expected) in rows {
        for (level, masked) in levels.iter().zip(expected) {
            let found = !hits("Schweinsbraten", *level, candidate).is_empty();
            assert_eq!(found, *masked, "{candidate:?} at {level:?}");
        }
    }
}

#[test]
fn masks_task_fuzzy_custom_budgets_are_used_as_set() {
    // Trim 3 without inner errors finds „Weinsbraten“ but not „Weinbraten“.
    let custom = Tolerance::Custom {
        trim: 3,
        inner_tenths: 0,
    };
    assert_eq!(
        hits("Schweinsbraten", custom, "Weinsbraten"),
        ["Weinsbraten"]
    );
    assert!(hits("Schweinsbraten", custom, "Weinbraten").is_empty());
    let wide = Tolerance::Custom {
        trim: 3,
        inner_tenths: 10,
    };
    assert_eq!(hits("Schweinsbraten", wide, "Weinbraten"), ["Weinbraten"]);
}

#[test]
fn masks_task_fuzzy_matches_never_borrow_neighbour_letters() {
    let text = "Heute gibt es Weinbraten mit Knoedel.";
    assert_eq!(
        hits("Schweinsbraten", Tolerance::Medium, text),
        ["Weinbraten"]
    );
    let text = "Der Schwein Braten war gut.";
    assert_eq!(
        hits("Schweinsbraten", Tolerance::Medium, text),
        ["Schwein Braten"]
    );
    assert!(hits("Schweinsbraten", Tolerance::Medium, "Dazu ein Weinbrand.").is_empty());
}

#[test]
fn masks_task_fuzzy_masks_exact_folded_occurrences_inside_compounds() {
    let text = "Die Schweinsbratensoße und KALBSSCHWEINSBRATEN, dazu Schweins-\nbratenreste.";
    assert_eq!(
        hits("Schweinsbraten", Tolerance::Off, text),
        ["Schweinsbraten", "SCHWEINSBRATEN", "Schweins-\nbraten"]
    );
}

#[test]
fn masks_task_fuzzy_entries_with_punctuation_match_across_it() {
    let text = "Termin bei Dr. Müller und DR MUELLER.";
    assert_eq!(
        hits("Dr. Müller", Tolerance::Low, text),
        ["Dr. Müller", "DR MUELLER"]
    );
}

#[test]
fn masks_task_fuzzy_judge_reports_the_reference_inner_costs() {
    let pattern = FuzzyPattern::new("Schweinsbraten", Tolerance::Medium).unwrap();
    let fold = |text: &str| super::fold::fold_pattern(text);
    // Costs in tenths: Weinbraten 1.0, OCR e→c 0.4, two OCR 0.8.
    assert_eq!(pattern.judge(&fold("Weinbraten")), Some(10));
    assert_eq!(pattern.judge(&fold("Schweinsbratcn")), Some(4));
    assert_eq!(pattern.judge(&fold("Schwcinsbratcn")), Some(8));
    assert_eq!(pattern.judge(&fold("Schweinshaxe")), None);
}
