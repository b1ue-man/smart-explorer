//! The desktop question for one unsigned update (E12, r-lokal-3; suite
//! expectation E12-gui of the review 2026-11-07): an unsigned release makes
//! the update check ask, the question blocks the Explorer shortcuts, and
//! cancelling starts nothing. The fixture app needs
//! `SMART_EXPLORER_COPY_PASTE_TASK=1`, which crates-task sets for every test.
use eframe::egui;

use super::UnsignedUpdatePrompt;
use crate::app::App;
use crate::updater::{UnsignedFeed, UpdateMsg};

const FRAME_KEYBOARD: &str = include_str!("frame_keyboard.rs");

fn prompt(feed: UnsignedFeed) -> UnsignedUpdatePrompt {
    UnsignedUpdatePrompt {
        version: "1.2.3".into(),
        feed,
        rollback: false,
    }
}

fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1200.0, 900.0),
        )),
        events,
        ..Default::default()
    };
    let _ = ctx.run(input, |ctx| app.ui_unsigned_update_dialog(ctx));
}

fn escape() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn quiet_app() -> App {
    let mut app = App::new_for_copy_task();
    app.update_rx = None;
    app.rollback_rx = None;
    app.update_ready = None;
    app.unsigned_update_prompt = None;
    app.notice = None;
    app.error_msg = None;
    app
}

/// An unsigned release makes the update check ask: the question names the
/// version and why it is unsigned, no update is ready, nothing else changes.
#[test]
fn lokal_task_unsigned_update_message_opens_the_question() {
    for feed in [UnsignedFeed::PlainHttp, UnsignedFeed::OfficialRelease] {
        let mut app = quiet_app();
        let (sender, receiver) = crossbeam_channel::unbounded();
        sender
            .send(UpdateMsg::UnsignedConsentNeeded {
                version: "1.2.3".into(),
                feed,
            })
            .expect("the check's answer");
        app.update_rx = Some(receiver);
        app.drain_update();
        let asked = app.unsigned_update_prompt.as_ref().expect("the question");
        assert_eq!(asked.version, "1.2.3", "{feed:?}");
        assert_eq!(asked.feed, feed);
        assert!(!asked.rollback, "{feed:?}");
        assert!(app.update_ready.is_none(), "{feed:?}");
        assert!(app.update_rx.is_none(), "the answer is consumed: {feed:?}");
        assert!(app.error_msg.is_none(), "{feed:?}");
    }
}

/// Escape cancels: the question closes, the user is told, nothing is started
/// (no check, no rollback download, no error).
#[test]
fn lokal_task_cancelling_the_unsigned_question_starts_nothing() {
    let ctx = egui::Context::default();
    let mut app = quiet_app();
    app.unsigned_update_prompt = Some(prompt(UnsignedFeed::PlainHttp));
    frame(&ctx, &mut app, Vec::new());
    assert!(
        app.unsigned_update_prompt.is_some(),
        "the question stays open without an answer"
    );
    frame(&ctx, &mut app, vec![escape()]);
    assert!(app.unsigned_update_prompt.is_none());
    assert!(app.update_rx.is_none() && app.rollback_rx.is_none());
    assert!(app.update_ready.is_none());
    assert!(app.error_msg.is_none());
    let (text, _) = app.notice.as_ref().expect("the user is told");
    assert!(
        text.contains("1.2.3") && text.contains("nicht geladen"),
        "{text}"
    );
}

/// A confirmation while a check already runs starts no second one.
#[test]
fn lokal_task_confirming_while_a_check_runs_starts_no_second_check() {
    let mut app = quiet_app();
    let (_sender, receiver) = crossbeam_channel::unbounded::<UpdateMsg>();
    app.update_rx = Some(receiver);
    app.start_unsigned_update(prompt(UnsignedFeed::PlainHttp));
    assert!(app.update_rx.is_some());
    assert!(app.rollback_rx.is_none());
    let (text, _) = app.notice.as_ref().expect("the user is told");
    assert!(text.contains("läuft bereits"), "{text}");
}

/// While the question is open the Explorer shortcuts must not act on the
/// file list behind it. `blocks_explorer_shortcuts` is private to
/// `frame_keyboard.rs`, so the test reads its source.
#[test]
fn lokal_task_open_question_blocks_the_explorer_shortcuts() {
    let start = FRAME_KEYBOARD
        .find("fn blocks_explorer_shortcuts(")
        .expect("the function exists");
    let body = &FRAME_KEYBOARD[start..];
    let body = &body[..body.find("\n    }\n").expect("the function ends")];
    assert!(
        body.contains("self.unsigned_update_prompt.is_some()"),
        "{body}"
    );
}
