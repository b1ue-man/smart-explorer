//! A batch's injection in calls (round 12, S1): its text goes to the
//! platform in calls of at most `TEXT_CALL_CHARACTERS` characters, one
//! after the other on the blocking pool. A session that gives up the
//! control or ends stops the injection before its next call
//! (`Injection::cancel`), so a long text (64 KiB of a batch, 40 ms per
//! character on X11's spare key) no longer holds every session's input of
//! its target for up to 44 minutes; the keeper waits for one call at most
//! (`worker_limits::INJECTION_CALL_LIMIT`). The calls that went
//! (`Injection::done`) count as pressed when the batch fails or stops.
//! Only a sink's injection is cut: it blocks while the host process types.
//! A source without a sink (Android) hands the batch to its queue without
//! waiting (`bridge_input_queue.rs`), which holds at most `HARD_LIMIT`
//! batches, so its batch stays one call.
use super::Injection;
use crate::screen::core::host_backend::{ScreenHostBackend, ScreenInputSink};
use crate::screen::core::input_model::InputEvent;
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::core::worker_limits::TEXT_CALL_CHARACTERS;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// `events` in calls whose text has at most `TEXT_CALL_CHARACTERS`
/// characters each, in their order; a batch within the bound is one call
/// of its events unchanged. A text is cut between characters, never inside
/// `\r\n` (one Return, `x11_keymap::typed_chars`, which a call then holds
/// as its last two characters).
pub(super) fn calls(events: &[InputEvent]) -> Vec<Vec<InputEvent>> {
    let mut calls = vec![Vec::new()];
    let mut room = TEXT_CALL_CHARACTERS;
    for event in events {
        let InputEvent::Text { text } = event else {
            if let Some(call) = calls.last_mut() {
                call.push(event.clone());
            }
            continue;
        };
        let mut rest = text.as_str();
        while !rest.is_empty() {
            if room == 0 {
                calls.push(Vec::new());
                room = TEXT_CALL_CHARACTERS;
            }
            let (piece, after) = rest.split_at(cut(rest, room));
            room = room.saturating_sub(piece.chars().count());
            if let Some(call) = calls.last_mut() {
                call.push(InputEvent::Text { text: piece.into() });
            }
            rest = after;
        }
    }
    calls
}

/// The byte index after at most `chars` characters of `text`, one more
/// when the cut would fall between the `\r` and `\n` of one Return.
fn cut(text: &str, chars: usize) -> usize {
    let end = text
        .char_indices()
        .nth(chars)
        .map_or(text.len(), |(at, _)| at);
    if text[..end].ends_with('\r') && text[end..].starts_with('\n') {
        end + 1
    } else {
        end
    }
}

impl Injection {
    /// Starts the injection of `events` with `sink` (the backend without
    /// one), call by call, on the blocking pool.
    pub(in super::super) fn start(
        events: &[InputEvent],
        (sink, backend): (Option<Arc<dyn ScreenInputSink>>, Arc<dyn ScreenHostBackend>),
        (reference, presented): (u64, bool),
        epoch: u64,
    ) -> Self {
        let calls: Arc<[Vec<InputEvent>]> = match sink {
            Some(_) => calls(events).into(),
            None => vec![events.to_vec()].into(),
        };
        let (cancel, done) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicUsize::new(0)),
        );
        let job = (calls.clone(), cancel.clone(), done.clone());
        let handle = tokio::task::spawn_blocking(move || {
            let (calls, cancel, done) = job;
            for call in calls.iter() {
                // Stopped: the rest of the batch does not go (a refusal no
                // viewer hears, the session gives the control up).
                if cancel.load(Ordering::Acquire) {
                    return Err(ControlBlock::ControlUnavailable);
                }
                match &sink {
                    Some(sink) => sink.inject(call, reference, presented),
                    None => backend.inject(call),
                }?;
                done.fetch_add(1, Ordering::Release);
            }
            Ok(())
        });
        Self {
            handle,
            events: events.to_vec(),
            calls,
            done,
            cancel,
            reference,
            presented,
            started: Instant::now(),
            epoch,
        }
    }

    /// The events of the calls that went in whole.
    pub(in super::super) fn went(&self) -> impl Iterator<Item = &InputEvent> {
        let done = self.done.load(Ordering::Acquire).min(self.calls.len());
        self.calls[..done].iter().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::calls;
    use crate::screen::core::input_model::InputEvent;
    use crate::screen::core::worker_limits::TEXT_CALL_CHARACTERS;

    fn text(text: &str) -> InputEvent {
        InputEvent::Text { text: text.into() }
    }

    /// Round 12 (S1): a batch within the bound stays one call unchanged;
    /// a longer text goes in calls of at most `TEXT_CALL_CHARACTERS`
    /// characters, in order, other events with the text around them, and
    /// `\r\n` is never cut apart.
    #[test]
    fn screen_task_input_text_goes_in_bounded_calls() {
        let key = InputEvent::Key {
            hid_usage: 0x04,
            down: true,
        };
        let short = vec![key.clone(), text("abc")];
        assert_eq!(calls(&short), vec![short.clone()]);
        let long = "ä".repeat(TEXT_CALL_CHARACTERS * 2 + 3);
        let split = calls(&[key.clone(), text(&long), key.clone()]);
        assert_eq!(split.len(), 3);
        let joined: String = split
            .iter()
            .flatten()
            .filter_map(|event| match event {
                InputEvent::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(joined, long);
        for call in &split {
            let characters: usize = call
                .iter()
                .map(|event| match event {
                    InputEvent::Text { text } => text.chars().count(),
                    _ => 0,
                })
                .sum();
            assert!(characters <= TEXT_CALL_CHARACTERS);
        }
        assert_eq!(split[0][0], key);
        assert_eq!(split[2].last(), Some(&key));
        // The cut would fall after the `\r` of a `\r\n`: the call takes both.
        let line = format!("{}\r\nx", "a".repeat(TEXT_CALL_CHARACTERS - 1));
        let split = calls(&[text(&line)]);
        let head = format!("{}\r\n", "a".repeat(TEXT_CALL_CHARACTERS - 1));
        assert_eq!(split, vec![vec![text(&head)], vec![text("x")]]);
        let before = "b".repeat(TEXT_CALL_CHARACTERS - 1);
        let split = calls(&[text(&before), text("\r\nz")]);
        assert_eq!(
            split,
            vec![vec![text(&before), text("\r\n")], vec![text("z")]]
        );
    }
}
