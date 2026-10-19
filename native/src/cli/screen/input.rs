//! `se screen input DEVICE [--source QUELLE] (--text TEXT | --key TASTE |
//! --click X,Y | --move X,Y | --scroll DX,DY | --action NAME) [--json]`
//! (SCR-M6): one input on another device's screen. The command opens a
//! session in the mode `control` (the device needs „Bildschirm steuern“
//! for this device, and its user may be asked first; the command then waits
//! and says so on stderr), sends the input and ends the session.
//! Coordinates: a number is a pixel of the source, a number with `%` a
//! share of it (`50%,50%` is the middle). `--key` names keys of
//! `se screen probe`'s toolbar and of the HID table (`a`, `enter`, `f5`,
//! `ctrl_left+c`: pressed in order, released backwards). Ctrl+Alt+Delete is
//! never sent. JSON `{"delivered","blocked"}`; `blocked` is
//! `elevated_window`, `secure_desktop`, `control_unavailable` or
//! `unsupported_on_host`, and the exit code is 1 then. No input is ever
//! logged.
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use clap::Args;
use clap_complete::ArgValueCandidates;
use serde_json::json;

use crate::daemon::{screen_stream, StreamEvent, StreamRequest};
use crate::screen::core::input_model::{key_usage, InputEvent, PointerButton, SystemAction};
use crate::screen::core::negotiate::CodecOffer;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::window_list::SourceId;
use crate::screen::core::wire_control::{ControlBlock, HostFrame, ViewerFrame};
use crate::screen::core::wire_hello::SessionMode;
use crate::share::access_grant::Capability;
use crate::share::PeerOpenTarget;

/// Time the host has to name the source size and to answer the input.
const ANSWER_WAIT: Duration = Duration::from_secs(5);
/// How long a blocked input may take to come back.
const BLOCK_WAIT: Duration = Duration::from_millis(1500);

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

/// One coordinate: a pixel of the source or a share of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Coord {
    Pixel(u32),
    Percent(f64),
}

/// What one call sends.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum InputAction {
    Text(String),
    Keys(Vec<u16>),
    Click(Coord, Coord),
    Move(Coord, Coord),
    Scroll(i16, i16),
    Action(SystemAction),
}

fn coord(text: &str) -> Result<Coord, String> {
    let text = text.trim();
    if let Some(share) = text.strip_suffix('%') {
        let value: f64 = share
            .trim()
            .parse()
            .map_err(|_| format!("{text}: no percentage"))?;
        if !(0.0..=100.0).contains(&value) {
            return Err(format!("{text}: percentages are 0–100"));
        }
        return Ok(Coord::Percent(value));
    }
    text.parse()
        .map(Coord::Pixel)
        .map_err(|_| format!("{text}: neither a pixel nor a percentage"))
}

fn pair(text: &str) -> Result<(&str, &str), String> {
    text.split_once(',')
        .ok_or_else(|| format!("{text}: X,Y expected"))
}

/// The action of the given options.
pub(crate) fn action_of(
    text: Option<&str>,
    key: Option<&str>,
    click: Option<&str>,
    move_to: Option<&str>,
    scroll: Option<&str>,
    action: Option<&str>,
) -> Result<InputAction, String> {
    if let Some(text) = text {
        if text.is_empty() {
            return Err("--text is empty".into());
        }
        return Ok(InputAction::Text(text.into()));
    }
    if let Some(key) = key {
        let usages = key
            .split('+')
            .map(|name| {
                key_usage(&name.trim().to_ascii_lowercase())
                    .ok_or_else(|| format!("unknown key {name:?}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(InputAction::Keys(usages));
    }
    if let Some(click) = click {
        let (x, y) = pair(click)?;
        return Ok(InputAction::Click(coord(x)?, coord(y)?));
    }
    if let Some(target) = move_to {
        let (x, y) = pair(target)?;
        return Ok(InputAction::Move(coord(x)?, coord(y)?));
    }
    if let Some(scroll) = scroll {
        let (dx, dy) = pair(scroll)?;
        let notches = |text: &str| -> Result<i16, String> {
            let value: i16 = text
                .trim()
                .parse()
                .map_err(|_| format!("{text}: no number"))?;
            value
                .checked_mul(120)
                .ok_or_else(|| format!("{text}: too many notches"))
        };
        return Ok(InputAction::Scroll(notches(dx)?, notches(dy)?));
    }
    if let Some(action) = action {
        let parsed: SystemAction = serde_json::from_value(json!(action))
            .map_err(|_| format!("unknown action {action:?}"))?;
        return Ok(InputAction::Action(parsed));
    }
    Err("one of --text, --key, --click, --move, --scroll or --action is needed".into())
}

/// `coord` as 0–65535 over a side of `length` pixels.
fn scaled(coord: Coord, length: Option<u32>) -> Result<u16, String> {
    let share = match coord {
        Coord::Percent(value) => value / 100.0,
        Coord::Pixel(pixel) => {
            let length = length
                .filter(|length| *length > 0)
                .ok_or("the source size is unknown; use percentages")?;
            if pixel >= length {
                return Err(format!("{pixel} lies outside the source ({length} pixels)"));
            }
            f64::from(pixel) / f64::from((length - 1).max(1))
        }
    };
    Ok((share * 65_535.0).round().clamp(0.0, 65_535.0) as u16)
}

/// The events of `action` for a source of `size`.
pub(crate) fn events(
    action: &InputAction,
    size: Option<FrameSize>,
) -> Result<Vec<InputEvent>, String> {
    let point = |x: Coord, y: Coord| -> Result<InputEvent, String> {
        Ok(InputEvent::PointerAbs {
            x: scaled(x, size.map(|size| size.width))?,
            y: scaled(y, size.map(|size| size.height))?,
        })
    };
    let button = |down| InputEvent::Button {
        button: PointerButton::Left,
        down,
    };
    Ok(match action {
        InputAction::Text(text) => vec![InputEvent::Text { text: text.clone() }],
        InputAction::Keys(usages) => {
            let downs = usages.iter().map(|usage| InputEvent::Key {
                hid_usage: *usage,
                down: true,
            });
            let ups = usages.iter().rev().map(|usage| InputEvent::Key {
                hid_usage: *usage,
                down: false,
            });
            downs.chain(ups).collect()
        }
        InputAction::Click(x, y) => vec![point(*x, *y)?, button(true), button(false)],
        InputAction::Move(x, y) => vec![point(*x, *y)?],
        InputAction::Scroll(dx, dy) => vec![InputEvent::Wheel { dx: *dx, dy: *dy }],
        InputAction::Action(action) => vec![InputEvent::Action { action: *action }],
    })
}

fn needs_size(action: &InputAction) -> bool {
    matches!(
        action,
        InputAction::Click(Coord::Pixel(_), _)
            | InputAction::Click(_, Coord::Pixel(_))
            | InputAction::Move(Coord::Pixel(_), _)
            | InputAction::Move(_, Coord::Pixel(_))
    )
}

/// What the device did with the input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Delivered {
    pub delivered: bool,
    pub blocked: Option<ControlBlock>,
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
) -> Result<Delivered, String> {
    let request = StreamRequest {
        mode: SessionMode::Control,
        source: source.clone(),
        decoders: vec![CodecOffer::tiles()],
        display_size: None,
        max_fps: 1,
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
            return Ok(Delivered {
                delivered: false,
                blocked: Some(ControlBlock::UnsupportedOnHost),
            });
        }
        let mut size = match &source {
            Some(SourceId::Monitor(id)) => hello.monitors.iter().find(|m| &m.id == id),
            None => hello
                .monitors
                .iter()
                .find(|m| m.primary)
                .or(hello.monitors.first()),
            _ => None,
        }
        .map(|monitor| FrameSize::new(monitor.width, monitor.height))
        .filter(|size| size.width > 0 && size.height > 0);
        if size.is_none() && needs_size(action) {
            let deadline = Instant::now() + ANSWER_WAIT;
            while size.is_none() && Instant::now() < deadline {
                if let Some(StreamEvent::Record { header, .. }) =
                    next(Some(deadline - Instant::now()))?
                {
                    size = Some(header.size);
                }
            }
        }
        let batch = events(action, size)?;
        control
            .send(&ViewerFrame::Input(batch))
            .map_err(|error| error.to_string())?;
        let deadline = Instant::now() + BLOCK_WAIT;
        while Instant::now() < deadline {
            match next(Some(deadline - Instant::now()))? {
                Some(StreamEvent::Host(HostFrame::ControlState(state)))
                    if state.blocked.is_some() =>
                {
                    return Ok(Delivered {
                        delivered: false,
                        blocked: state.blocked,
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
        Ok(Delivered {
            delivered: true,
            blocked: None,
        })
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
    let delivered = deliver(
        choice.target,
        source,
        &action,
        None,
        super::pending_notice(choice.name),
    )?;
    let blocked = delivered
        .blocked
        .map(|blocked| serde_json::to_value(blocked).unwrap_or_default());
    let value = json!({ "delivered": delivered.delivered, "blocked": blocked });
    super::output::print(args.json, &value, || match &blocked {
        None => "delivered\n".to_string(),
        Some(reason) => format!("blocked\t{}\n", reason.as_str().unwrap_or_default()),
    });
    Ok(if delivered.delivered { 0 } else { 1 })
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
