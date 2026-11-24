//! `se screen input DEVICE [--source QUELLE] (--text TEXT | --key TASTE |
//! --click X,Y | --move X,Y | --scroll DX,DY | --action NAME) [--json]`
//! (SCR-M6): one input on another device's screen. The command opens a
//! session in the mode `control` (the device needs „Bildschirm steuern“
//! for this device, and its user may be asked first; the command then waits
//! and says so on stderr), sends the input and ends the session.
//! Coordinates: a number is a pixel of the source (its own size, also of a
//! window sent scaled down; a host that does not name a window's size takes
//! percent only), a number with `%` a share of it (`50%,50%` is the middle). `--key` names keys of
//! `se screen probe`'s toolbar and of the HID table (`a`, `enter`, `f5`,
//! `ctrl_left+c`: pressed in order, released backwards). Ctrl+Alt+Delete is
//! never sent. The input goes out once the session has received a complete
//! picture (the host takes input only after the viewer acknowledged a picture
//! of its current reference). JSON `{"sent","applied","blocked","held",
//! "refused","stopped"}`: `sent` says the input left this device and the
//! device neither blocked, held back, refused nor stopped it. A device that
//! sends input receipts (`input_receipt.rs`, round 12 rest) answers once the
//! input is settled, on an Android device once its app performed or refused
//! it, and the command waits for that answer as long as the session lasts,
//! at most `receipt_wait` (round 13, S4): `applied` is then `true` or
//! `false`; without a receipt within that time `applied` is `null` and
//! `unconfirmed` names why (`sent` stays `true`). An older device sends no receipt:
//! its answer is waited for 1.5 s (3.8 s on an Android device) and `applied`
//! is `null`, so `sent` is unconfirmed. `blocked` is `elevated_window`, `secure_desktop`,
//! `control_unavailable` or `unsupported_on_host`; `held` says the host held
//! the input back without applying it (`viewer_hidden`,
//! `waiting_for_picture`, `release_pending`, `backlog_full`, `other`), or that no complete
//! picture came within five seconds, so the input was not sent
//! (`waiting_for_picture`); `refused` is the device's sentence for an input
//! it took but did not perform (an Android host: a mouse button without
//! counterpart, a field whose content it cannot see); `stopped`
//! (`{"untyped","exact","text","block","held"}`) says the control went or the
//! session ended before the text was typed, or (with `block` or `held`, round
//! 13 S2) that a block or hold stopped it after part of it was typed, with
//! the untyped characters (`exact: false`: at most that many). With any of
//! them the exit code is 1. No input is ever logged.
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use clap::Args;
use clap_complete::ArgValueCandidates;

use crate::daemon::{screen_stream, StreamEvent, StreamRequest};
#[cfg(test)]
use crate::screen::core::input_model::{InputEvent, SystemAction};
use crate::screen::core::input_receipt::{text_characters, INPUT_RECEIPTS_V1};
use crate::screen::core::negotiate::CodecOffer;
#[cfg(test)]
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::{ControlBlock, HostFrame, InputHeld, ViewerFrame};
use crate::screen::core::wire_hello::{HostPlatform, SessionMode};
use crate::screen::core::worker_limits::{
    INPUT_TIMEOUT, TEXT_CALL_CHARACTERS, TEXT_CHARACTER_TIME,
};
use crate::share::access_grant::Capability;
use crate::share::PeerOpenTarget;

/// Time the host has to name the source size and to answer the input.
const ANSWER_WAIT: Duration = Duration::from_secs(5);
/// How long a blocked input may take to come back.
const BLOCK_WAIT: Duration = Duration::from_millis(1500);
/// What an Android host may take on top of that before it says that it did
/// not perform an input (round 12, S9 / Codex 12/15): once the wait for its
/// accessibility service (`AccessLink.INPUT_CONNECT_WAIT_MS`, 1 s; the host
/// leaves the rest of the batch after it) and per stroke its duration plus
/// the grace for Android's answer (`ScreenHostControl.STROKE_GRACE_MS`,
/// 500 ms); the largest input of this command, a scroll, makes two strokes of
/// 150 ms (`GestureMapper`). Before, a refusal after 1.5 s was reported as
/// `sent`.
const ANDROID_ANSWER: Duration = Duration::from_millis(1000 + 2 * (150 + 500));

/// How long the answer to a sent input is waited for on a host of
/// `platform`.
fn answer_wait(platform: HostPlatform) -> Duration {
    match platform {
        HostPlatform::Android => BLOCK_WAIT + ANDROID_ANSWER,
        _ => BLOCK_WAIT,
    }
}

/// How long the receipt of `batch` is waited for on a host of `platform`
/// (round 13, S4: a lost receipt held the command for the whole session):
/// the answer of an older host (`answer_wait`), `ANSWER_WAIT` for the way
/// there and back [assumption: transport, not measured], and the host's own
/// limits for the batch (`worker_limits`): per text call of at most
/// `TEXT_CALL_CHARACTERS` characters the wait for the host process's link and
/// its answer time (`INPUT_TIMEOUT` each), plus `TEXT_CHARACTER_TIME` per
/// character.
fn receipt_wait(
    platform: HostPlatform,
    batch: &[crate::screen::core::input_model::InputEvent],
) -> Duration {
    let characters = text_characters(batch);
    let calls = characters.div_ceil(TEXT_CALL_CHARACTERS as u64).max(1);
    let times = |count: u64| u32::try_from(count).unwrap_or(u32::MAX);
    (answer_wait(platform) + ANSWER_WAIT)
        .saturating_add(INPUT_TIMEOUT.saturating_mul(times(calls.saturating_mul(2))))
        .saturating_add(TEXT_CHARACTER_TIME.saturating_mul(times(characters)))
}

#[derive(Args, Debug, PartialEq, Eq)]
pub(crate) struct InputArgs {
    #[arg(
        help = "Device: share:// endpoint, name, id or fingerprint prefix",
        add = ArgValueCandidates::new(crate::cli::completions_screen::device_candidates)
    )]
    device: String,
    /// Source: monitor:<id> or window:<id> (default: the primary monitor)
    #[arg(long, value_name = "QUELLE")]
    source: Option<String>,
    /// Text for the focused field
    #[arg(long, group = "what", value_name = "TEXT")]
    text: Option<String>,
    /// Key or key combination, e.g. enter or ctrl_left+c
    #[arg(long, group = "what", value_name = "TASTE")]
    key: Option<String>,
    /// Left click at X,Y (pixels or percent)
    #[arg(long, group = "what", value_name = "X,Y")]
    click: Option<String>,
    /// Move the pointer to X,Y (pixels or percent)
    #[arg(long = "move", group = "what", value_name = "X,Y")]
    move_to: Option<String>,
    /// Scroll by DX,DY notches (up and right positive)
    #[arg(long, group = "what", value_name = "DX,DY", allow_hyphen_values = true)]
    scroll: Option<String>,
    /// System action (Android): back, home, recents, notifications,
    /// quick_settings, lock_screen, power_dialog
    #[arg(long, group = "what", value_name = "NAME")]
    action: Option<String>,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[path = "input_action.rs"]
mod action;
#[path = "input_answer.rs"]
mod answer;
pub(crate) use action::{action_of, events, InputAction};
use action::{hello_size, needs_size};
pub(crate) use answer::outcome_value;
use answer::{receipt_outcome, unconfirmed_outcome};

/// What became of the input: `sent` when it left this device and the
/// device neither blocked, held back, refused nor stopped it. `applied`:
/// the device's receipt (`input_receipt.rs`) says it applied the input
/// (`Some(true)`) or not (`Some(false)`); `None` for a device without
/// receipts, whose `sent` is then unconfirmed (its answer was waited for
/// `answer_wait`). `held`: the host dropped the batch without applying it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InputOutcome {
    pub sent: bool,
    pub applied: Option<bool>,
    pub blocked: Option<ControlBlock>,
    pub held: Option<InputHeld>,
    /// The device took the input but did not perform it (its sentence, an
    /// Android host's notice: a mouse button without counterpart, a field
    /// whose content it cannot see, a refused text or action).
    pub refused: Option<String>,
    /// The control went or the session ended before the text was typed,
    /// or a block or hold stopped it partway (round 13, S2): the untyped
    /// characters, whether that number is exact, and that block or hold.
    pub stopped: Option<Stopped>,
    /// No receipt came within `receipt_wait` (round 13, S4): why `applied`
    /// is `null` although the device announced receipts.
    pub unconfirmed: Option<String>,
}

/// What a `stopped` receipt says (`InputResult::Stopped`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stopped {
    pub untyped: u64,
    pub exact: bool,
    pub block: Option<ControlBlock>,
    pub held: Option<InputHeld>,
}

impl InputOutcome {
    /// Not sent: `blocked`, `held` or neither (set by the caller).
    fn not_sent() -> Self {
        Self {
            sent: false,
            applied: None,
            blocked: None,
            held: None,
            refused: None,
            stopped: None,
            unconfirmed: None,
        }
    }
}

/// Opens a control session to `target`, sends `action` and ends it.
/// `accept_within` bounds the wait for the device's user (MCP); `notice`
/// hears that the user is asked.
pub(crate) fn deliver(
    target: PeerOpenTarget,
    source: Option<SourceId>,
    action: &InputAction,
    accept_within: Option<Duration>,
    mut notice: impl FnMut(Capability),
) -> Result<InputOutcome, String> {
    let request = StreamRequest {
        expected_node: None,
        frame_capabilities: None,
        mode: SessionMode::Control,
        source: source.clone(),
        decoders: vec![CodecOffer::tiles()],
        display_size: None,
        max_fps: 1,
        max_rate_bps: None,
        codec: None,
        decoder: None,
        // Input only: no picture is decoded.
        gop_continue: false,
        gop_continue_decoders: Vec::new(),
        virtual_display: None,
    };
    let mut stream = screen_stream(target, request)?;
    let control = stream.control();
    let (sender, received) = mpsc::channel();
    std::thread::Builder::new()
        .name("screen-input-reader".into())
        .spawn(move || loop {
            let event = stream.next_event();
            let last = matches!(event, Ok(StreamEvent::End(_)) | Err(_));
            if sender.send(event).is_err() || last {
                return;
            }
        })
        .map_err(|error| error.to_string())?;
    let next = |wait: Option<Duration>| -> Result<Option<StreamEvent>, String> {
        let event = match wait {
            Some(wait) => match received.recv_timeout(wait) {
                Err(RecvTimeoutError::Timeout) => return Ok(None),
                other => other.map_err(|_| "the screen session ended".to_string()),
            },
            None => received
                .recv()
                .map_err(|_| "the screen session ended".to_string()),
        }?;
        event.map(Some).map_err(|error| error.to_string())
    };
    let outcome = (|| {
        let asked = Instant::now();
        let hello = loop {
            let wait = accept_within.map(|limit| limit.saturating_sub(asked.elapsed()));
            match next(wait)? {
                None => {
                    return Err("approval_pending: the device's user did not answer in time".into())
                }
                Some(StreamEvent::Pending(capability)) => notice(capability),
                Some(StreamEvent::Accepted { hello, .. }) => break hello,
                Some(StreamEvent::End(end)) => {
                    return Err(format!(
                        "{}{}",
                        end.code,
                        end.detail.map(|d| format!(": {d}")).unwrap_or_default()
                    ))
                }
                Some(_) => {}
            }
        };
        let supported = match action {
            InputAction::Text(_) => hello.input.text,
            InputAction::Keys(_) => hello.input.keyboard,
            InputAction::Click(..) | InputAction::Move(..) | InputAction::Scroll(..) => {
                hello.input.pointer || hello.input.touch
            }
            InputAction::Action(action) => hello.input.actions.contains(action),
        };
        if !supported {
            return Ok(InputOutcome {
                blocked: Some(ControlBlock::UnsupportedOnHost),
                ..InputOutcome::not_sent()
            });
        }
        let mut size = hello_size(&hello, &source);
        let answer = answer_wait(hello.platform);
        // The host drops input silently until the viewer acknowledged a
        // picture of its current reference (`host_session_control.rs`,
        // `input`); the worker acknowledges every picture it receives, so
        // the input waits for the first complete one (at most `ANSWER_WAIT`).
        // Without one it is not sent (review 2026-11-19, Codex 3/20): a host
        // would drop it, so the answer says `held: waiting_for_picture`.
        // While the host waits for its user's consent to share the screen
        // (Android's MediaProjection dialog: `waiting_for_portal`), the wait
        // for the user's answer applies (`accept_within`, none for the CLI),
        // not the picture time (Gemini 3/19).
        // Pixels are pixels of the source: only the size the host names for
        // it converts them (`HELLO_OK` monitors, or the source size of the
        // stream in `STATE`). A picture's size is what the host sent,
        // possibly scaled down, so a pixel of the source would land elsewhere
        // (Codex 4/16).
        let mut deadline = Some(Instant::now() + ANSWER_WAIT);
        let mut picture = false;
        let mut stream_reported = false;
        // A pixel position waits for the stream's report too: the source may
        // have changed its size since `HELLO_OK` (Codex 5/16).
        while !picture || (needs_size(action) && !stream_reported) {
            let wait = match deadline {
                Some(deadline) => match deadline.checked_duration_since(Instant::now()) {
                    Some(left) if !left.is_zero() => Some(left),
                    _ => break,
                },
                None => None,
            };
            match next(wait)? {
                // The daemon rebuilt the host session (Gemini 3/9.4): the new
                // one has its own pictures and sizes, so the wait starts over.
                Some(StreamEvent::Accepted { hello: again, .. }) => {
                    picture = false;
                    stream_reported = false;
                    size = hello_size(&again, &source);
                    deadline = Some(Instant::now() + ANSWER_WAIT);
                }
                Some(StreamEvent::Record { record, .. }) => picture |= !record.has_more(),
                Some(StreamEvent::Host(HostFrame::State(report))) => {
                    if let Some(stream) = &report.stream {
                        stream_reported = true;
                        // The source size of the running stream comes first.
                        size = stream.source_size.or(size);
                    }
                    deadline = if report.state == SessionState::WaitingForPortal {
                        accept_within.map(|limit| Instant::now() + limit)
                    } else {
                        deadline.or(Some(Instant::now() + ANSWER_WAIT))
                    };
                }
                Some(StreamEvent::End(end)) => return Err(end.code),
                _ => {}
            }
        }
        if !picture {
            return Ok(InputOutcome {
                held: Some(InputHeld::WaitingForPicture),
                ..InputOutcome::not_sent()
            });
        }
        // A pixel position needs the size of the running stream's source: the
        // size from `HELLO_OK` may be out of date (Codex 6/17). Only a stream
        // report without a source size (a host before it) leaves the hello's.
        if needs_size(action) && !stream_reported {
            return Err(
                "the device has not reported its running picture stream in time, so the size \
                 of the source is unknown; give the position in percent, e.g. 50%,50%"
                    .into(),
            );
        }
        if size.is_none() && needs_size(action) {
            // A host before the source size in `STATE` names none for a window.
            return Err(
                "the device does not name the size of this source (a window's picture may \
                 be scaled); give the position in percent, e.g. 50%,50%"
                    .into(),
            );
        }
        let batch = events(action, size)?;
        let receipt_limit = receipt_wait(hello.platform, &batch);
        // Round 12 rest (Codex 12/15): a host that announced receipts answers
        // this input once it is settled (on an Android host: once its app
        // performed or refused it), however long that takes; the session's
        // end ends the wait. An older host sends none: its answer is waited
        // for `answer_wait` and the result stays unconfirmed (`applied: null`).
        let receipts = hello.input_receipts == Some(INPUT_RECEIPTS_V1);
        if receipts {
            control
                .send(&ViewerFrame::InputReceipts(true))
                .map_err(|error| error.to_string())?;
        }
        control
            .send(&ViewerFrame::Input(batch))
            .map_err(|error| error.to_string())?;
        if receipts {
            return receipt_outcome(&next, receipt_limit);
        }
        unconfirmed_outcome(&next, answer)
    })();
    let _ = control.close();
    outcome
}

pub(super) fn run(args: InputArgs) -> Result<i32, String> {
    let action = action_of(
        args.text.as_deref(),
        args.key.as_deref(),
        args.click.as_deref(),
        args.move_to.as_deref(),
        args.scroll.as_deref(),
        args.action.as_deref(),
    )?;
    let source = args
        .source
        .as_deref()
        .map(SourceId::parse)
        .transpose()
        .map_err(|error| format!("--source: {error}"))?;
    let choice = super::select_device(&args.device)?;
    let outcome = deliver(
        choice.target,
        source,
        &action,
        None,
        super::pending_notice(choice.name),
    )?;
    let value = answer::outcome_value(&outcome);
    super::output::print(args.json, &value, || answer::outcome_line(&value));
    Ok(if outcome.sent { 0 } else { 1 })
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
