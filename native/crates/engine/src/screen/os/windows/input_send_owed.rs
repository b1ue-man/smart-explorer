//! `SendInput` with partial insertion (review 2026-11-19, C14, Codex 2/8):
//! a press that went in without its release is let go again, and releases
//! Windows refused as well stay owed until they are in.
use super::*;

pub(super) fn submit(inputs: &[INPUT]) -> Result<(), ControlBlock> {
    // Releases owed from an earlier partial batch go first; while Windows
    // refuses them, no new input goes in behind them.
    let owed = std::mem::take(&mut state().owed);
    if !owed.is_empty() {
        let done = send(&owed).min(owed.len());
        if done < owed.len() {
            let mut state = state();
            let newer = std::mem::take(&mut state.owed);
            state.owed = owed[done..].to_vec();
            state.owed.extend(newer);
            return Err(ControlBlock::ElevatedWindow);
        }
    }
    if inputs.is_empty() {
        return Ok(());
    }
    let sent = send(inputs).min(inputs.len());
    if sent == inputs.len() {
        return Ok(());
    }
    // Windows refuses input it blocks (UIPI) without saying more.
    if sent == 0 {
        return Err(ControlBlock::ElevatedWindow);
    }
    // Part of the batch went in (review 2026-11-19, C14): the session takes
    // over a batch only whole, so a press that went in without its release
    // is let go here, newest first; the rest of the batch is dropped and
    // the batch counts as not applied. Windows names no cause (SendInput:
    // "neither GetLastError nor the return value will indicate the failure
    // was caused by UIPI blocking"); the refusal is reported as the zero
    // count is, at once and as Windows' own refusal (`elevated_window`), not
    // as `control_unavailable`, which the session may take for a stale
    // reference and hold back once (`stale_refusal`, review N5).
    // Codex 2/8: what of these releases Windows refuses as well stays owed
    // and goes before the next batch, until it is in.
    let releases = unreleased(&inputs[..sent]);
    let done = send(&releases).min(releases.len());
    state().owed.extend_from_slice(&releases[done..]);
    Err(ControlBlock::ElevatedWindow)
}

/// `SendInput` "returns the number of events that it successfully inserted
/// into the keyboard or mouse input stream".
fn send(inputs: &[INPUT]) -> usize {
    if inputs.is_empty() {
        return 0;
    }
    // SAFETY: `inputs` holds `len` initialized records of `size_of::<INPUT>()`.
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    sent as usize
}

/// The releases of the keys and buttons `inserted` pressed and did not let
/// go again, newest first.
fn unreleased(inserted: &[INPUT]) -> Vec<INPUT> {
    // What identifies a key: code, scan code and the flags besides `KEYUP`.
    const KEY_FLAGS: u32 = KEYEVENTF_SCANCODE | KEYEVENTF_EXTENDEDKEY | KEYEVENTF_UNICODE;
    let mut held: Vec<INPUT> = Vec::new();
    for input in inserted {
        if input.r#type == INPUT_KEYBOARD {
            // SAFETY: `ki` is the member a keyboard record carries.
            let ki = unsafe { input.Anonymous.ki };
            let release = key(ki.wVk, ki.wScan, (ki.dwFlags & KEY_FLAGS) | KEYEVENTF_KEYUP);
            let same = |entry: &INPUT| {
                // SAFETY: `held` holds keyboard and mouse records by their type.
                entry.r#type == INPUT_KEYBOARD
                    && unsafe {
                        let other = entry.Anonymous.ki;
                        (other.wVk, other.wScan, other.dwFlags & KEY_FLAGS)
                            == (ki.wVk, ki.wScan, ki.dwFlags & KEY_FLAGS)
                    }
            };
            held.retain(|entry| !same(entry));
            if ki.dwFlags & KEYEVENTF_KEYUP == 0 {
                held.push(release);
            }
        } else if input.r#type == INPUT_MOUSE {
            // SAFETY: `mi` is the member a mouse record carries.
            let mi = unsafe { input.Anonymous.mi };
            let Some((up, down)) = [
                (MOUSEEVENTF_LEFTUP, MOUSEEVENTF_LEFTDOWN),
                (MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_RIGHTDOWN),
                (MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MIDDLEDOWN),
                (MOUSEEVENTF_XUP, MOUSEEVENTF_XDOWN),
            ]
            .into_iter()
            .find(|(up, down)| mi.dwFlags & (up | down) != 0) else {
                continue;
            };
            let data = mi.mouseData as i32;
            let same = |entry: &INPUT| {
                // SAFETY: `held` holds keyboard and mouse records by their type.
                entry.r#type == INPUT_MOUSE
                    && unsafe {
                        let other = entry.Anonymous.mi;
                        other.dwFlags == up
                            && (up != MOUSEEVENTF_XUP || other.mouseData as i32 == data)
                    }
            };
            held.retain(|entry| !same(entry));
            if mi.dwFlags & down != 0 {
                held.push(mouse(up, 0, 0, data));
            }
        }
    }
    held.reverse();
    held
}
