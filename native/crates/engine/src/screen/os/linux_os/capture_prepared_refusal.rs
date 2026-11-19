//! A portal batch the portal refused (review 2026-11-19, S2, S5): what it
//! pressed is let go, the next batch reports the refusal, and a new portal
//! identity starts without the state of the one before.
use std::sync::Mutex;

/// A refused batch the session took as applied (`send_input` queued it):
/// the releases of what the batch pressed before `failed` and did not let
/// go, newest first, and every release from `failed` on (review
/// 2026-11-19, S2; the session holds those keys for released).
pub(super) fn portal_releases(
    batch: &[crate::screen::core::input_model::InputEvent],
    failed: usize,
) -> Vec<crate::screen::core::input_model::InputEvent> {
    use crate::screen::core::input_model::{InputEvent, TouchPhase};
    let release = |event: &InputEvent| match event {
        InputEvent::Key { hid_usage, .. } => Some(InputEvent::Key {
            hid_usage: *hid_usage,
            down: false,
        }),
        InputEvent::Button { button, .. } => Some(InputEvent::Button {
            button: *button,
            down: false,
        }),
        InputEvent::Touch { id: 0, x, y, .. } => Some(InputEvent::Touch {
            id: 0,
            phase: TouchPhase::Cancel,
            x: *x,
            y: *y,
        }),
        _ => None,
    };
    let same = |a: &InputEvent, b: &InputEvent| match (a, b) {
        (InputEvent::Touch { .. }, InputEvent::Touch { .. }) => true,
        _ => a == b,
    };
    let mut held: Vec<InputEvent> = Vec::new();
    for event in &batch[..failed.min(batch.len())] {
        let down = match event {
            InputEvent::Key { down, .. } | InputEvent::Button { down, .. } => *down,
            InputEvent::Touch {
                id: 0,
                phase: TouchPhase::Move,
                ..
            } => continue,
            InputEvent::Touch { id: 0, phase, .. } => *phase == TouchPhase::Start,
            _ => continue,
        };
        let Some(up) = release(event) else {
            continue;
        };
        held.retain(|entry| !same(entry, &up));
        if down {
            held.push(up);
        }
    }
    held.reverse();
    for event in batch.iter().skip(failed) {
        if crate::screen::os::portal_input::release_only(std::slice::from_ref(event))
            && !held
                .iter()
                .any(|entry| same(entry, event) && entry == event)
        {
            held.push(event.clone());
        }
    }
    held
}

/// A portal refusal the session has not heard yet; the next batch gets it
/// as its block (S2). One host process injects for one session.
pub(super) static PORTAL_REFUSED: Mutex<Option<crate::screen::core::wire_control::ControlBlock>> =
    Mutex::new(None);

/// Releases the portal refused (Codex 3/11): they go before the next batch.
static PORTAL_OWED: Mutex<Vec<crate::screen::core::input_model::InputEvent>> =
    Mutex::new(Vec::new());

/// The next batch reports `block` (S2).
pub(super) fn refuse(block: crate::screen::core::wire_control::ControlBlock) {
    *PORTAL_REFUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(block);
}

/// The next batch reports the portal's refusal (`UnsupportedOnHost`).
pub(super) fn refuse_unsupported() {
    refuse(crate::screen::core::wire_control::ControlBlock::UnsupportedOnHost);
}

/// `left` stays owed; the next batch reports the refusal (Codex 3/11).
pub(super) fn owe(left: Vec<crate::screen::core::input_model::InputEvent>) {
    if left.is_empty() {
        return;
    }
    let mut refused = PORTAL_REFUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    refused.get_or_insert(crate::screen::core::wire_control::ControlBlock::UnsupportedOnHost);
    drop(refused);
    PORTAL_OWED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .extend(left);
}

/// Keysyms of a text whose release the portal refused (Codex 4/18).
static PORTAL_OWED_SYMS: Mutex<Vec<i32>> = Mutex::new(Vec::new());

/// `sym` was pressed and its release refused: it stays owed, and the next
/// batch reports the refusal.
pub(super) fn owe_sym(sym: i32) {
    PORTAL_REFUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert(crate::screen::core::wire_control::ControlBlock::UnsupportedOnHost);
    PORTAL_OWED_SYMS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(sym);
}

/// Releases the owed keysyms; `false` (the rest owed again) when the portal
/// refuses one of them again.
pub(super) async fn release_owed_syms(
    remote: &ashpd::desktop::remote_desktop::RemoteDesktop,
    session: &ashpd::desktop::Session<ashpd::desktop::remote_desktop::RemoteDesktop>,
) -> bool {
    let syms = std::mem::take(
        &mut *PORTAL_OWED_SYMS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    for (index, sym) in syms.iter().enumerate() {
        let up = crate::screen::os::portal_input::key_state(false);
        if remote
            .notify_keyboard_keysym(session, *sym, up, Default::default())
            .await
            .is_err()
        {
            let mut owed = PORTAL_OWED_SYMS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            owed.splice(0..0, syms[index..].iter().copied());
            return false;
        }
    }
    true
}

pub(super) fn take_owed() -> Vec<crate::screen::core::input_model::InputEvent> {
    std::mem::take(
        &mut *PORTAL_OWED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

/// A new portal identity was bound: no refusal and no wheel rest of the
/// one before carry over (S2, S5).
pub(in crate::screen::os) fn reset_portal_input() {
    *PORTAL_REFUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    PORTAL_OWED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    PORTAL_OWED_SYMS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    *super::WHEEL_REST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = (0, 0);
}
