use super::{take_clipboard_keys, ClipKey};

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_keyboard_blocked_queue_is_consumed_not_replayed() {
    assert_eq!(std::env::var("SMART_EXPLORER_COPY_PASTE_TASK").as_deref(), Ok("1"));
    let (tx, rx) = crossbeam_channel::unbounded();
    tx.send(ClipKey::Copy).unwrap();
    tx.send(ClipKey::Paste).unwrap();
    tx.send(ClipKey::Cut).unwrap();
    // update_keyboard always takes the queue before a modal/typing guard and
    // discards these actions when file shortcuts are blocked in that frame.
    let blocked_frame = take_clipboard_keys(Some(&rx));
    assert_eq!(blocked_frame, ([true, true, true], false));
    assert_eq!(take_clipboard_keys(Some(&rx)), ([false; 3], false));
    tx.send(ClipKey::Paste).unwrap();
    assert_eq!(take_clipboard_keys(Some(&rx)), ([false, false, true], false));
    drop(tx);
    assert_eq!(take_clipboard_keys(Some(&rx)), ([false; 3], true));
    assert_eq!(take_clipboard_keys(None), ([false; 3], false));
}
