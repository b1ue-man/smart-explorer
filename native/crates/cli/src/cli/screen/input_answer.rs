//! The device's answer to the input of `se screen input` and of the MCP tool
//! `screen_input`: its receipt (`input_receipt.rs`, round 12 rest, Codex
//! 12/15) from a host that announced receipts, waited for at most
//! `receipt_wait` (round 13, S4), else what an older host reports within
//! `answer_wait` (unconfirmed).
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{InputOutcome, Stopped};
use crate::daemon::StreamEvent;
use crate::screen::core::input_receipt::InputResult;
use crate::screen::core::wire_control::HostFrame;

/// Reads the next event; `None` once `wait` passed (`None`: no limit).
pub(super) type Next<'a> = dyn Fn(Option<Duration>) -> Result<Option<StreamEvent>, String> + 'a;

/// A rebuilt host session while the input went out: the relay drops
/// control frames during the rebuild, so whether it was applied is unknown;
/// it is not sent twice (a second click).
const REBUILT: &str = "the device rebuilt the screen session while the input went out; it may \
                       not have been applied";

/// The receipt of the first `INPUT` after `INPUT_RECEIPTS`, waited for at
/// most `wait` (the session's end ends it earlier); without one by then the
/// input is `sent` with `applied: null` and the reason in `unconfirmed`
/// (round 13, S4: a lost receipt held the command until the session ended).
pub(super) fn receipt_outcome(next: &Next<'_>, wait: Duration) -> Result<InputOutcome, String> {
    let deadline = Instant::now() + wait;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let event = if left.is_zero() {
            None
        } else {
            next(Some(left))?
        };
        let Some(event) = event else {
            return Ok(InputOutcome {
                sent: true,
                unconfirmed: Some(format!(
                    "the device sent no receipt for the input within {:.1} s; whether it was \
                     applied is unknown",
                    wait.as_secs_f64()
                )),
                ..InputOutcome::not_sent()
            });
        };
        match event {
            StreamEvent::Host(HostFrame::ControlState(state)) => {
                let Some(receipt) = state.input.filter(|receipt| receipt.batch == 1) else {
                    continue;
                };
                let mut outcome = InputOutcome {
                    applied: Some(false),
                    ..InputOutcome::not_sent()
                };
                match receipt.result {
                    InputResult::Applied => {
                        outcome.sent = true;
                        outcome.applied = Some(true);
                    }
                    InputResult::Blocked { block } => outcome.blocked = Some(block),
                    InputResult::Held { held } => outcome.held = Some(held),
                    InputResult::Refused { reason } => outcome.refused = Some(reason),
                    InputResult::Stopped {
                        untyped,
                        exact,
                        block,
                        held,
                    } => {
                        outcome.stopped = Some(Stopped {
                            untyped,
                            exact,
                            block,
                            held,
                        })
                    }
                    InputResult::Other => {
                        return Err(
                            "the device answered the input with a result this version does \
                             not know"
                                .into(),
                        )
                    }
                }
                return Ok(outcome);
            }
            StreamEvent::Accepted { .. } => return Err(REBUILT.into()),
            StreamEvent::Host(HostFrame::Error(error)) => {
                return Err(format!(
                    "{}: {}",
                    error.code,
                    error.detail.unwrap_or_default()
                ))
            }
            StreamEvent::End(end) => return Err(end.code),
            _ => {}
        }
    }
}

/// An older host's answer within `answer`: a block, a hold or an Android
/// refusal it reports; otherwise `sent` without confirmation.
pub(super) fn unconfirmed_outcome(
    next: &Next<'_>,
    answer: Duration,
) -> Result<InputOutcome, String> {
    let deadline = Instant::now() + answer;
    while Instant::now() < deadline {
        match next(Some(deadline - Instant::now()))? {
            // The device answered that it did not perform the input (an
            // Android host's notice, no block: round 11, M3).
            Some(StreamEvent::Host(HostFrame::State(report))) => {
                if let Some(refused) = report
                    .reason
                    .as_deref()
                    .and_then(crate::screen::core::codec_reason_text::input_refusal)
                {
                    return Ok(InputOutcome {
                        refused: Some(refused.to_string()),
                        ..InputOutcome::not_sent()
                    });
                }
            }
            Some(StreamEvent::Accepted { .. }) => return Err(REBUILT.into()),
            // Held back (E2): not applied. Its `blocked` only repeats the
            // block reported before this batch, so `held` decides.
            Some(StreamEvent::Host(HostFrame::ControlState(state))) if state.held.is_some() => {
                return Ok(InputOutcome {
                    held: state.held,
                    ..InputOutcome::not_sent()
                })
            }
            Some(StreamEvent::Host(HostFrame::ControlState(state))) if state.blocked.is_some() => {
                return Ok(InputOutcome {
                    blocked: state.blocked,
                    ..InputOutcome::not_sent()
                })
            }
            Some(StreamEvent::Host(HostFrame::Error(error))) => {
                return Err(format!(
                    "{}: {}",
                    error.code,
                    error.detail.unwrap_or_default()
                ))
            }
            Some(StreamEvent::End(end)) => return Err(end.code),
            _ => {}
        }
    }
    Ok(InputOutcome {
        sent: true,
        ..InputOutcome::not_sent()
    })
}

/// The answer as JSON (`se screen input --json`, MCP `screen_input`).
pub(crate) fn outcome_value(outcome: &InputOutcome) -> Value {
    let stopped = outcome.stopped.as_ref().map(|stopped| {
        json!({
            "untyped": stopped.untyped,
            "exact": stopped.exact,
            "block": stopped.block,
            "held": stopped.held,
            "text": crate::screen::core::input_receipt::stopped_result_text(
                stopped.untyped,
                stopped.exact,
                stopped.block,
                stopped.held,
            ),
        })
    });
    json!({
        "sent": outcome.sent,
        "applied": outcome.applied,
        "blocked": outcome.blocked,
        "held": outcome.held,
        "refused": outcome.refused,
        "stopped": stopped,
        "unconfirmed": outcome.unconfirmed,
    })
}

/// The answer as one line: `held`, `blocked`, `refused` or `stopped` with
/// its reason, else `applied` (receipt), `sent` with the reason in
/// `unconfirmed` (no receipt in time) or `sent` (no receipt).
pub(super) fn outcome_line(value: &Value) -> String {
    let text = |key: &str| value[key].as_str().map(str::to_string);
    if let Some(held) = text("held") {
        return format!("held\t{held}\n");
    }
    if let Some(blocked) = text("blocked") {
        return format!("blocked\t{blocked}\n");
    }
    if let Some(refused) = text("refused") {
        return format!("refused\t{refused}\n");
    }
    if let Some(stopped) = value["stopped"]["text"].as_str() {
        return format!("stopped\t{stopped}\n");
    }
    if value["applied"] == Value::Bool(true) {
        return "applied\n".into();
    }
    if let Some(unconfirmed) = text("unconfirmed") {
        format!("sent\t{unconfirmed}\n")
    } else {
        "sent\n".into()
    }
}
