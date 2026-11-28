//! Language choice, spans, checkpoints and long lines of the highlighter.
use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};

use crate::textview::{
    Highlighter, StyledLine, ThemeKind, LINE_STRIDE, MAX_HIGHLIGHT_CHARS, PLAIN_TEXT,
};

fn builtin() -> Arc<Highlighter> {
    static SHARED: OnceLock<Arc<Highlighter>> = OnceLock::new();
    Arc::clone(SHARED.get_or_init(|| Arc::new(Highlighter::builtin())))
}

fn assert_covering(line: &StyledLine) {
    let mut at = 0;
    for span in &line.spans {
        assert_eq!(span.start, at, "spans without gaps: {line:?}");
        assert!(span.end > span.start);
        at = span.end;
    }
    assert_eq!(at, line.text.len(), "spans cover the line: {line:?}");
}

#[test]
fn textview_task_language_detection() {
    let hl = builtin();
    let cases = [
        ("Program.cs", "", "C#"),
        ("index.html", "", "HTML"),
        ("src/main.rs", "", "Rust"),
        ("C:\\Projekt\\README.md", "", "Markdown"),
        ("Makefile", "", "Makefile"),
        ("Dockerfile", "", "Dockerfile"),
        ("Dockerfile.dev", "", "Dockerfile"),
        ("CMakeLists.txt", "", "CMake"),
        ("werkzeug", "#!/usr/bin/env python3", "Python"),
        ("daten", "<?xml version=\"1.0\"?>", "XML"),
        ("notizen.unbekannt", "nur Text", PLAIN_TEXT),
    ];
    for (name, first_line, expected) in cases {
        assert_eq!(hl.detect(name, first_line), expected, "{name}");
    }
    assert!(hl.detect("start", "#!/bin/bash").contains("bash"));
    assert_eq!(hl.language_by_name("rust").as_deref(), Some("Rust"));
    assert_eq!(hl.language_by_name("cs").as_deref(), Some("C#"));
    assert_eq!(hl.language_by_name("C#").as_deref(), Some("C#"));
    assert_eq!(hl.language_by_name("gibtsnicht"), None);
    let names: BTreeSet<String> = hl.languages().into_iter().map(|l| l.name).collect();
    assert!(names.contains("TOML") && names.contains("TypeScript"));
    // An unknown session language falls back to plain text.
    assert_eq!(hl.session("gibtsnicht").language(), PLAIN_TEXT);
}

#[test]
fn textview_task_highlight_spans() {
    let hl = builtin();
    let samples = [
        ("C#", "public class Foo { private int x = 42; // Kommentar"),
        ("HTML", "<div class=\"rahmen\">Hallo <b>Welt</b></div>"),
        ("Rust", "fn main() { let x: u32 = 1; println!(\"{x}\"); }"),
    ];
    for (language, code) in samples {
        let lines = vec![code.to_string()];
        let session = hl.session(language);
        assert_eq!(session.language(), language);
        let light = session
            .highlight(lines.as_slice(), 0, 1, ThemeKind::Light)
            .unwrap();
        assert_eq!(light.len(), 1);
        assert_covering(&light[0]);
        let colors: BTreeSet<[u8; 4]> = light[0].spans.iter().map(|s| s.style.fg).collect();
        assert!(colors.len() >= 2, "{language}: several token colors");
        let dark = session
            .highlight(lines.as_slice(), 0, 1, ThemeKind::Dark)
            .unwrap();
        assert_ne!(light, dark, "{language}: the theme changes the colors");
    }
    assert_ne!(
        hl.theme_colors(ThemeKind::Light).background,
        hl.theme_colors(ThemeKind::Dark).background
    );
    // Plain text: one span in the theme's text color.
    let plain = hl.highlight_text(PLAIN_TEXT, "nur Text", ThemeKind::Light);
    assert_eq!(plain[0].spans.len(), 1);
    assert_eq!(plain[0].spans[0].style, hl.default_style(ThemeKind::Light));
}

/// Rust source with block comments and strings crossing stride boundaries.
fn long_rust_source() -> Vec<String> {
    let mut lines = Vec::new();
    for i in 0..3500u64 {
        let line = match i % 700 {
            695 => "/* Kommentar über".to_string(),
            699 => "   mehrere Zeilen */ let s = \"x\";".to_string(),
            _ if i % 7 == 0 => format!("fn f{i}() -> u64 {{ {i} }}"),
            _ => format!("    let v{i} = \"text {i}\"; // {i}"),
        };
        lines.push(line);
    }
    // A comment open across the first checkpoint boundary.
    lines[1020] = "/*".to_string();
    lines[1030] = "*/".to_string();
    lines
}

#[test]
fn textview_task_checkpoints_match_top_down() {
    let hl = builtin();
    let lines = long_rust_source();
    let top_down = hl
        .session("Rust")
        .highlight(lines.as_slice(), 0, lines.len(), ThemeKind::Dark)
        .unwrap();
    assert_eq!(top_down.len(), lines.len());
    // A jump into the middle: parsed from the start, checkpoints stored.
    let jump = hl.session("Rust");
    let window = jump
        .highlight(lines.as_slice(), 2000, 50, ThemeKind::Dark)
        .unwrap();
    assert_eq!(window.as_slice(), &top_down[2000..2050]);
    assert!(jump.checkpoint_count() >= 2);
    // Background preparation, then windows from the nearest checkpoint.
    let prepared = hl.session("Rust");
    let covered = prepared
        .prepare(lines.as_slice(), None, &AtomicBool::new(false), &mut |_| {})
        .unwrap();
    assert_eq!(covered, lines.len() as u64);
    assert_eq!(
        prepared.checkpoint_count() as u64,
        lines.len() as u64 / LINE_STRIDE + 1
    );
    for first in [1025u64, 2048, 3070] {
        let window = prepared
            .highlight(lines.as_slice(), first, 20, ThemeKind::Dark)
            .unwrap();
        let first = first as usize;
        let end = (first + 20).min(lines.len());
        assert_eq!(window.as_slice(), &top_down[first..end], "from {first}");
    }
    // Inside the comment the line has the comment style, as from the top.
    assert_eq!(top_down[1025].spans.len(), 1);
}

#[test]
fn textview_task_long_line_plain() {
    let hl = builtin();
    let long = "a".repeat(MAX_HIGHLIGHT_CHARS + 1);
    let lines = vec![long.clone(), "fn x() {}".to_string()];
    let out = hl
        .session("Rust")
        .highlight(lines.as_slice(), 0, 2, ThemeKind::Light)
        .unwrap();
    assert_eq!(out[0].spans.len(), 1);
    assert_eq!(out[0].spans[0].style, hl.default_style(ThemeKind::Light));
    assert_eq!(out[0].text, long);
    // The long line leaves the parser state unchanged.
    let alone = hl.highlight_text("Rust", "fn x() {}", ThemeKind::Light);
    assert_eq!(out[1], alone[0]);
    assert!(out[1].spans.len() >= 2);
}
