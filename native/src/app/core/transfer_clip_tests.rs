//! The explorer's clipboard entry: what makes it current, cut, and the text
//! marker the Linux build compares a paste with.
use super::super::transfer_route::{TransferPlace, TransferSelection};
use super::*;

fn selection(paths: &[&str]) -> TransferSelection {
    TransferSelection::roots(
        TransferPlace::local(),
        paths.iter().map(|path| path.to_string()).collect(),
        None,
    )
}

#[test]
fn transfer_engine_task_clip_is_current_under_its_sequences_only() {
    let mut clip = AppClip::new(selection(&["/data/a"]), false).guarded_by_sequence(Some(5));
    assert!(clip.is_current(Some(5), None));
    assert!(!clip.is_current(Some(6), None), "another program copied");
    // Our own publication (CF_HDROP, virtual files) moves the sequence on.
    clip.add_sequence(7);
    assert!(clip.is_current(Some(7), None));
    assert!(clip.is_current(Some(5), None));
    assert!(!clip.is_current(Some(8), None));
    // An unreadable sequence cannot prove a change: our entry wins.
    assert!(clip.is_current(None, None));
    // Text a paste brings is irrelevant where sequences decide.
    assert!(clip.is_current(Some(7), Some("irgendein Text")));
    assert!(clip.take_pending_marker().is_none());
}

#[test]
fn transfer_engine_task_clip_without_sequence_stays_current_until_replaced() {
    let mut clip = AppClip::new(selection(&["/data/a"]), false).guarded_by_sequence(None);
    assert!(clip.is_current(Some(1), None));
    assert!(clip.is_current(None, Some("x")));
    clip.add_sequence(3);
    assert!(clip.is_current(Some(3), None));
    assert!(!clip.is_current(Some(4), None));
}

#[test]
fn transfer_engine_task_clip_marker_decides_on_linux() {
    let marker = clip_marker_text(&["/home/u/a b.txt".to_string(), "/home/u/dir".to_string()]);
    assert_eq!(marker, "/home/u/a b.txt\n/home/u/dir");
    let mut clip = AppClip::new(selection(&["/home/u/a b.txt", "/home/u/dir"]), false)
        .guarded_by_marker(marker.clone());
    // Written once to the system clipboard by the next frame.
    assert_eq!(clip.take_pending_marker(), Some(marker));
    assert_eq!(clip.take_pending_marker(), None);
    // Ctrl+V brings the text back (the windowing layer may turn line ends).
    assert!(clip.is_current(None, Some("/home/u/a b.txt\r\n/home/u/dir\n")));
    assert!(
        !clip.is_current(None, Some("kopierter Text")),
        "another program copied"
    );
    // Menu commands carry no clipboard text: the entry is used.
    assert!(clip.is_current(None, None));
    // An empty marker cannot guard anything.
    let unguarded = AppClip::new(selection(&["/x"]), false).guarded_by_marker(String::new());
    assert!(unguarded.is_current(None, Some("anything")));
}

#[test]
fn transfer_engine_task_clip_cut_moves_and_says_so() {
    let copied = AppClip::new(selection(&["/data/a", "/data/b"]), false);
    assert!(!copied.cut);
    assert_eq!(copied.count(), 2);
    assert!(copied.copied_notice().starts_with("✓ 2 Element(e) kopiert"));
    assert!(copied
        .copied_notice()
        .contains("Strg+V startet die Übertragung"));
    let cut = AppClip::new(selection(&["/data/a"]), true);
    assert!(cut.cut);
    assert!(cut.copied_notice().contains("ausgeschnitten"));
    assert!(cut.copied_notice().contains("verschiebt"));
}
