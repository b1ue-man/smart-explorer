//! The stream masker: output split at any byte gives the same masked
//! stream as masking it whole; nothing of a match is emitted early; the
//! held rest is released at the end and after silence.
use std::time::{Duration, Instant};

use super::mask_set::MaskSet;
use super::mask_set_tests::set_of;
use super::model::{NewEntry, Tolerance};
use super::stream::STREAM_IDLE_RELEASE;

fn entries() -> Vec<NewEntry> {
    vec![
        NewEntry::literal("Mustermann").with_label("Name"),
        NewEntry::fuzzy("Schweinsbraten", Tolerance::Medium).with_label("Speise"),
        NewEntry::regex(r"\d{3}-\d{4}").with_label("Akte"),
    ]
}

#[test]
fn masks_task_stream_over_chunk_boundaries_equals_whole_masking() {
    let set = set_of(entries());
    let mut text = String::new();
    for index in 0..1200 {
        text.push_str(&format!(
            "Zeile {index}: Herr Mustermann isst Schweins-\nbraten, Akte 123-{:04} ⟦x\n",
            index % 10_000
        ));
    }
    let whole = set.mask(&text).into_string();
    for chunk in [1usize, 3, 7, 4096, 70_000] {
        let mut stream = set.stream();
        let mut out = Vec::new();
        for piece in text.as_bytes().chunks(chunk) {
            out.extend(stream.push(piece));
        }
        out.extend(stream.flush());
        assert_eq!(String::from_utf8(out).unwrap(), whole, "chunk {chunk}");
    }
}

#[test]
fn masks_task_stream_never_emits_part_of_a_possible_match() {
    let set = set_of(entries());
    let mut stream = set.stream();
    let mut shown = Vec::new();
    for piece in ["Mus", "ter", "ma"] {
        shown.extend(stream.push(piece.as_bytes()));
    }
    let partial = String::from_utf8(shown.clone()).unwrap();
    assert!(!partial.contains("Mus"), "{partial:?}");
    shown.extend(stream.push(b"nn ok\n"));
    shown.extend(stream.flush());
    assert_eq!(String::from_utf8(shown).unwrap(), "⟦Name-1⟧ ok\n");
}

#[test]
fn masks_task_stream_releases_a_prompt_after_silence() {
    let set = set_of(entries());
    let mut stream = set.stream();
    assert!(stream.push(b"Passwort fuer Mustermann: ").is_empty());
    let deadline = stream.idle_deadline().unwrap();
    assert!(stream
        .release_idle_at(deadline - Duration::from_millis(1))
        .is_empty());
    let released = stream.release_idle_at(Instant::now() + STREAM_IDLE_RELEASE);
    assert_eq!(released, "Passwort fuer ⟦Name-1⟧: ".as_bytes());
    assert_eq!(stream.pending_len(), 0);
}

#[test]
fn masks_task_stream_of_an_empty_set_passes_bytes_through() {
    let set = std::sync::Arc::new(MaskSet::empty());
    let mut stream = set.stream();
    assert_eq!(stream.push(b"Mus"), b"Mus");
    assert!(stream.flush().is_empty());
}
