//! The text view without drawing: „Gehe zu Zeile“, searching in the file
//! (forward, backward, wrap-around, case, regex, over a document), the
//! window state over a document in memory and Markdown link resolution.
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::textview::TextDocument;

use super::content_search_nav::line_regex;
use super::md_links::{heading_slug, parent_dir, resolve_link, LinkTarget};
use super::text_view_find::{find_next, parse_goto, FindFrom, FindHit};
use super::text_view_window::{is_markdown, TextViewWindow, ViewState};

#[test]
fn text_view_task_goto_line_parses_and_checks_the_range() {
    assert_eq!(parse_goto("12", Some(100)), Ok(11));
    assert_eq!(parse_goto(" 1.234 ", Some(2000)), Ok(1233));
    assert_eq!(
        parse_goto("500", None),
        Ok(499),
        "before the index is complete any line is taken"
    );
    assert_eq!(
        parse_goto("0", Some(5)).unwrap_err(),
        "Die Zeilen beginnen bei 1."
    );
    assert_eq!(
        parse_goto("", Some(5)).unwrap_err(),
        "Bitte eine Zeilennummer eingeben."
    );
    assert!(parse_goto("zwölf", Some(5))
        .unwrap_err()
        .contains("keine Zeilennummer"));
    assert_eq!(
        parse_goto("101", Some(100)).unwrap_err(),
        "Die Datei hat nur 100 Zeilen."
    );
}

#[test]
fn text_view_task_find_in_file_forward_backward_case_regex_and_wrap() {
    let lines: Vec<String> = ["foo bar", "nothing", "Bar foo", "end"]
        .map(String::from)
        .to_vec();
    let cancel = AtomicBool::new(false);
    let foo = line_regex("foo", false, false).unwrap();
    let find = |matcher: &regex::Regex, line, byte, forward| {
        find_next(
            lines.as_slice(),
            matcher,
            FindFrom { line, byte },
            forward,
            4,
            &cancel,
        )
        .unwrap()
    };
    let at = |line, start, end| Some(FindHit { line, start, end });
    let first = find(&foo, 0, 0, true);
    assert_eq!((first.hit, first.wrapped), (at(0, 0, 3), false));
    assert_eq!(find(&foo, 0, 3, true).hit, at(2, 4, 7));
    let wrapped = find(&foo, 2, 7, true);
    assert_eq!((wrapped.hit, wrapped.wrapped), (at(0, 0, 3), true));
    assert_eq!(find(&foo, 2, 4, false).hit, at(0, 0, 3));
    let back = find(&foo, 0, 0, false);
    assert_eq!((back.hit, back.wrapped), (at(2, 4, 7), true));
    let bar_case = line_regex("Bar", false, true).unwrap();
    assert_eq!(
        find(&bar_case, 0, 0, true).hit,
        at(2, 0, 3),
        "match case skips „bar“"
    );
    let anchored = line_regex("^B", true, true).unwrap();
    assert_eq!(find(&anchored, 0, 0, true).hit, at(2, 0, 1));
    let missing = line_regex("zzz", false, false).unwrap();
    assert_eq!(find(&missing, 1, 0, true).hit, None);
    assert!(line_regex("(", true, false).is_err());
    // The same search over a document reads it in windows of lines.
    let doc = TextDocument::from_bytes(b"a\r\nneedle\r\nb".to_vec()).unwrap();
    let needle = line_regex("NEEDLE", false, false).unwrap();
    let found = find_next(
        &doc,
        &needle,
        FindFrom { line: 0, byte: 0 },
        true,
        1,
        &cancel,
    )
    .unwrap();
    assert_eq!(found.hit, at(1, 0, 6));
}

#[test]
fn text_view_task_markdown_links_resolve_relative_absolute_and_web() {
    let base = "/home/u/proj/docs";
    let file = |path: &str| LinkTarget::File(path.to_string());
    assert_eq!(resolve_link(base, "a.md"), file("/home/u/proj/docs/a.md"));
    assert_eq!(
        resolve_link(base, "../README.md#start"),
        file("/home/u/proj/README.md")
    );
    assert_eq!(
        resolve_link(base, "./my%20file.md?x=1"),
        file("/home/u/proj/docs/my file.md")
    );
    assert_eq!(resolve_link(base, "/etc/x.md"), file("/etc/x.md"));
    assert_eq!(
        resolve_link("/a", "../../../b.md"),
        file("/b.md"),
        "never above the root"
    );
    assert_eq!(
        resolve_link(base, "https://example.org/a"),
        LinkTarget::Web("https://example.org/a".into())
    );
    assert_eq!(
        resolve_link(base, "HTTP://X"),
        LinkTarget::Web("HTTP://X".into())
    );
    assert_eq!(
        resolve_link(base, "mailto:a@b.de"),
        LinkTarget::Web("mailto:a@b.de".into())
    );
    assert_eq!(resolve_link(base, "#top"), LinkTarget::Anchor("top".into()));
    assert_eq!(
        resolve_link(base, "javascript:alert(1)"),
        LinkTarget::Unsupported
    );
    assert_eq!(heading_slug(" Zwei Wörter: Ende! "), "zwei-wörter-ende");
    assert_eq!(resolve_link(base, "file:///C:/x/y.md"), file("C:/x/y.md"));
    assert_eq!(resolve_link(base, "file:///srv/z.md"), file("/srv/z.md"));
    assert_eq!(
        resolve_link("C:/Users/u/docs", "..\\b.md"),
        file("C:/Users/u/b.md")
    );
    assert_eq!(resolve_link(base, "D:/z.md"), file("D:/z.md"));
    assert_eq!(parent_dir("/a/b.md"), "/a");
    assert_eq!(parent_dir("/b.md"), "/");
    assert_eq!(
        resolve_link(&parent_dir("C:/x.md"), "y.md"),
        file("C:/y.md")
    );
    assert!(is_markdown("Notiz.MD") && is_markdown("a.markdown") && !is_markdown("a.rs"));
}

fn wait_until(
    window: &mut TextViewWindow,
    ctx: &egui::Context,
    done: impl Fn(&TextViewWindow) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done(window) {
        assert!(Instant::now() < deadline, "the view did not answer in time");
        std::thread::sleep(Duration::from_millis(10));
        window.poll(ctx);
    }
}

#[test]
fn text_view_task_window_index_goto_find_and_lines() {
    let ctx = egui::Context::default();
    let text = (1..=50)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\nneedle here\n";
    let doc = TextDocument::from_bytes(text.into_bytes()).unwrap();
    let mut window = TextViewWindow::from_document(1, "/x/a.txt", doc);
    wait_until(&mut window, &ctx, |w| {
        matches!(w.state, ViewState::Ready) && w.index_complete()
    });
    assert_eq!(window.known_lines(), 52);
    assert!(
        window.status_text().starts_with("UTF-8 · LF · 52 Zeilen"),
        "{}",
        window.status_text()
    );
    window.goto.text = "40".into();
    window.apply_goto();
    assert_eq!(
        (window.cursor, window.goto.error.as_deref()),
        (Some(39), None)
    );
    window.goto.text = "53".into();
    window.apply_goto();
    assert_eq!(
        window.goto.error.as_deref(),
        Some("Die Datei hat nur 52 Zeilen.")
    );
    window.find.text = "NEEDLE".into();
    window.start_find(true);
    wait_until(&mut window, &ctx, |w| {
        w.find.current.is_some()
            || w.find
                .status
                .as_deref()
                .is_some_and(|s| s.contains("nicht"))
    });
    assert_eq!(window.find.current.map(|hit| hit.line), Some(50));
    assert_eq!(window.cursor, Some(50));
    window.ensure_lines(48, 3);
    wait_until(&mut window, &ctx, |w| w.line(50).is_some());
    assert_eq!(
        window.line(50).map(|line| line.text.as_str()),
        Some("needle here")
    );
    window.scroll_to(u64::MAX);
    assert_eq!(window.top, window.max_top());
}
