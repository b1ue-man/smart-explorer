//! A portal batch the portal refused (review 2026-11-19, S2, S5): what it
//! pressed is let go, the next batch reports the refusal, and a new portal
//! identity starts without the state of the one before.
use std::sync::{Arc, Mutex};

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

/// One batch of the portal thread: owed releases first, then the batch
/// (`Batch::Input`), or the releases alone (`Batch::Release`, also empty).
pub(in crate::screen::os) async fn dispatch_portal_batch(
    remote: &ashpd::desktop::remote_desktop::RemoteDesktop,
    session: &ashpd::desktop::Session<ashpd::desktop::remote_desktop::RemoteDesktop>,
    identity: &Arc<crate::screen::os::pipewire_stream::identity::StreamIdentity>,
    path: &str,
    batch: crate::screen::os::portal_input::Batch,
    sas: &mut crate::screen::os::input_keys::SasGuard,
) {
    use crate::screen::os::portal_input::Batch;
    match batch {
        Batch::Input(events) => {
            // Owed releases first; refused again, this batch's go owed too (Codex 3/11).
            let owed = take_owed();
            if !release_owed_syms(remote, session).await {
                refuse_unsupported(); // E3: the batch went nowhere
                owe(owed);
                owe(portal_releases(&events, 0));
                return;
            }
            if !owed.is_empty() {
                let left = super::apply_portal_release(remote, session, &owed, sas).await;
                if !left.is_empty() {
                    owe(left);
                    owe(portal_releases(&events, 0));
                    return;
                }
            }
            if let Err((failed, block)) =
                super::apply_portal_input(remote, session, identity, path, &events, sas).await
            {
                // Never the events themselves: input is not logged.
                eprintln!(
                    "screen host process: the portal refused input ({block:?}); \
                     held keys of the batch are released"
                );
                let releases = portal_releases(&events, failed);
                let left = super::apply_portal_release(remote, session, &releases, sas).await;
                refuse(block);
                owe(left);
            }
        }
        Batch::Release {
            identity: bound,
            events,
        } if Arc::ptr_eq(&bound, identity) && bound.session == path => {
            release_owed_syms(remote, session).await;
            // Round 6 (R3): the releases owed from before go first.
            let mut all = take_owed();
            all.extend(events);
            let left = super::apply_portal_release(remote, session, &all, sas).await;
            if !left.is_empty() {
                eprintln!("screen host process: the portal refused a release");
            }
            owe(left);
        }
        Batch::Release { .. } => {}
    }
}
