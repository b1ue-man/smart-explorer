//! What `se screen input` sends: the options as one action and its input
//! events, the coordinates of the source (`input.rs`).
use serde_json::json;

use crate::screen::core::input_model::{key_usage, InputEvent, PointerButton, SystemAction};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::window_list::SourceId;

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

/// The size of `source` as `HELLO_OK` names it (monitors only).
pub(super) fn hello_size(
    hello: &crate::screen::core::wire_hello::ScreenHelloOk,
    source: &Option<SourceId>,
) -> Option<FrameSize> {
    match source {
        Some(SourceId::Monitor(id)) => hello.monitors.iter().find(|m| &m.id == id),
        None => hello
            .monitors
            .iter()
            .find(|m| m.primary)
            .or(hello.monitors.first()),
        _ => None,
    }
    .map(|monitor| FrameSize::new(monitor.width, monitor.height))
    .filter(|size| size.width > 0 && size.height > 0)
}

pub(super) fn needs_size(action: &InputAction) -> bool {
    matches!(
        action,
        InputAction::Click(Coord::Pixel(_), _)
            | InputAction::Click(_, Coord::Pixel(_))
            | InputAction::Move(Coord::Pixel(_), _)
            | InputAction::Move(_, Coord::Pixel(_))
    )
}
