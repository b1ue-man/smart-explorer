//! The MCP tool `screen` (SCR-M4): the devices whose screen this device may
//! see, what a device shows (platform, monitors, codecs, limits), the
//! windows it runs and one still picture as a PNG image. There is no live
//! video, no input and no way to answer a confirmation: a device that
//! asks its user keeps the call waiting for `wait` seconds, then the
//! session is ended and the call fails with `approval_pending` (the
//! confirmation is the human's, S1 C7).
//!
//! Input (SCR-M6) is the tool `screen_input` of its own, annotated as
//! changing and destructive, so a host that approves read-only tools on its
//! own (Codex) still asks before an agent clicks or types on another
//! device; the device needs „screen control“ for this device, and
//! Ctrl+Alt+Delete is never sent. The text of an input is not echoed in
//! the answer.
use super::args::Args;
use super::response;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::snapshot::{fit, Fitted};
use crate::screen::core::window_list::SourceId;
use base64::Engine as _;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{json, Value};
use std::time::Duration;

/// Long edge of a still picture unless the call says otherwise: longer
/// edges are shrunk by the image processing of the models themselves, so
/// more pixels only cost bytes.
const DEFAULT_MAX_EDGE: u64 = 1568;
/// Seconds to wait for the device's user unless the call says otherwise.
const DEFAULT_WAIT_SECS: u64 = 30;

pub(super) const DESCRIPTION: &str = "Looks at the screen of a paired device: `devices` lists the devices and their screen rights, `probe` shows platform, monitors, codecs and limits, `windows` lists the windows that run there, `families` returns confirmed versioned window-family selectors, `snapshot` returns one still picture (PNG image, long edge at most 1568 px unless `max_edge` says otherwise). The other device must have granted „screen view“; if it asks its user, the call waits up to `wait` seconds (default 30) and then fails with `approval_pending`. There is no live video and no way to answer a confirmation through this tool; input goes through the tool `screen_input`.";

const DEVICE: &str = "Device: a `share://direct/<id>` or `share://room/<room>/<device>` endpoint, a name, an id or a fingerprint prefix; `devices` lists them.";

pub(super) fn properties() -> Value {
    json!({
        "action": {
            "type": "string",
            "enum": ["devices", "probe", "windows", "families", "snapshot"],
            "description": "devices, probe, windows, confirmed families or snapshot."
        },
        "device": {"type": "string", "description": DEVICE},
        "source": {
            "type": "string",
            "description": "snapshot: what to show, `monitor:<id>` or `window:<id>` from `probe` or `windows` (default: the primary monitor)."
        },
        "probe": {
            "type": "boolean",
            "description": "devices: ask each online device for its rights and facts (default false)."
        },
        "max_edge": {
            "type": "integer",
            "minimum": 1,
            "description": "snapshot: longest edge of the picture in pixels (default 1568)."
        },
        "max_bytes": {
            "type": "integer",
            "minimum": 1,
            "description": "snapshot: shrink the picture until its PNG has at most this many bytes (the answer carries it as base64)."
        },
        "wait": {
            "type": "integer",
            "minimum": 0,
            "description": "Seconds to wait for the device's user to answer its question (default 30)."
        }
    })
}

fn wait(args: &Args) -> Result<Duration, String> {
    Ok(Duration::from_secs(
        args.opt_u64("wait")?.unwrap_or(DEFAULT_WAIT_SECS),
    ))
}

/// The answer of `snapshot`: one PNG image and the facts as text and as
/// structured content (without the picture).
pub(super) fn image_result(fitted: &Fitted, source: &str, device: &str) -> CallToolResult {
    let meta = json!({
        "width": fitted.size.width,
        "height": fitted.size.height,
        "bytes": fitted.png.len(),
        "source": source,
        "device": device,
    });
    let data = base64::engine::general_purpose::STANDARD.encode(&fitted.png);
    let mut result = CallToolResult::success(vec![
        ContentBlock::image(data, "image/png"),
        ContentBlock::text(meta.to_string()),
    ]);
    result.structured_content = Some(meta);
    result
}

fn snapshot(args: &Args) -> Result<CallToolResult, String> {
    let selector = args.string("device")?;
    let source = args
        .opt_string("source")?
        .map(|text| SourceId::parse(&text))
        .transpose()
        .map_err(|error| format!("argument `source`: {error}"))?;
    let edge = args.opt_u64("max_edge")?.unwrap_or(DEFAULT_MAX_EDGE);
    let edge = u32::try_from(edge)
        .ok()
        .filter(|edge| *edge > 0)
        .ok_or("argument `max_edge` must be between 1 and 4294967295")?;
    let max_bytes = args
        .opt_u64("max_bytes")?
        .map(|bytes| usize::try_from(bytes).unwrap_or(usize::MAX));
    let wait = wait(args)?;
    let (target, device) = crate::cli::screen::device_target(&selector)?;
    let still = crate::daemon::screen_snapshot_within(
        target,
        source,
        Some(FrameSize::new(edge, edge)),
        Some(wait),
        |_| {},
    )?;
    let fitted = fit(&still.image, Some(edge), max_bytes).map_err(|error| error.to_string())?;
    Ok(image_result(&fitted, &still.source, &device))
}

pub(super) const INPUT_DESCRIPTION: &str = "Clicks, moves the pointer, scrolls, types text, presses a key or a key combination, or runs an Android system action on the screen of a paired device (one input per call). The other device must have granted „screen control“ to this device; if it asks its user, the call waits up to `wait` seconds (default 30) and then fails with `approval_pending`. Coordinates are pixels of the source or percentages (`50%`). The answer says whether the input was delivered or why the device blocked it (`elevated_window`, `secure_desktop`, `control_unavailable`, `unsupported_on_host`). Ctrl+Alt+Delete is never sent.";

pub(super) fn input_properties() -> Value {
    json!({
        "device": {"type": "string", "description": DEVICE},
        "source": {
            "type": "string",
            "description": "`monitor:<id>` or `window:<id>` from the `screen` tool (default: the primary monitor)."
        },
        "text": {"type": "string", "description": "Text for the focused field."},
        "key": {"type": "string", "description": "A key or combination: `enter`, `f5`, `ctrl_left+c`."},
        "click": {"type": "string", "description": "Left click at `X,Y` (pixels or percentages)."},
        "move": {"type": "string", "description": "Move the pointer to `X,Y`."},
        "scroll": {"type": "string", "description": "Scroll by `DX,DY` notches (up and right positive)."},
        "action": {
            "type": "string",
            "enum": ["back", "home", "recents", "notifications", "quick_settings", "lock_screen", "power_dialog"],
            "description": "An Android system action."
        },
        "wait": {
            "type": "integer",
            "minimum": 0,
            "description": "Seconds to wait for the device's user to answer its question (default 30)."
        }
    })
}

/// The tool `screen_input`: one input, the answer `{"delivered","blocked"}`.
pub(super) fn screen_input(args: &Args) -> Result<CallToolResult, String> {
    let selector = args.string("device")?;
    let source = args
        .opt_string("source")?
        .map(|text| SourceId::parse(&text))
        .transpose()
        .map_err(|error| format!("argument `source`: {error}"))?;
    let text = args.opt_string("text")?;
    let key = args.opt_string("key")?;
    let click = args.opt_string("click")?;
    let move_to = args.opt_string("move")?;
    let scroll = args.opt_string("scroll")?;
    let action = args.opt_string("action")?;
    let given = [&text, &key, &click, &move_to, &scroll, &action]
        .iter()
        .filter(|value| value.is_some())
        .count();
    if given != 1 {
        return Err(
            "give exactly one of `text`, `key`, `click`, `move`, `scroll` or `action`".into(),
        );
    }
    let input = crate::cli::screen::input::action_of(
        text.as_deref(),
        key.as_deref(),
        click.as_deref(),
        move_to.as_deref(),
        scroll.as_deref(),
        action.as_deref(),
    )?;
    let wait = wait(args)?;
    let (target, device) = crate::cli::screen::device_target(&selector)?;
    let delivered = crate::cli::screen::input::deliver(target, source, &input, Some(wait), |_| {})?;
    Ok(response::ok(json!({
        "device": device,
        "delivered": delivered.delivered,
        "blocked": delivered.blocked,
    })))
}

pub(super) fn screen(args: &Args) -> Result<CallToolResult, String> {
    match args.string("action")?.as_str() {
        "devices" => Ok(response::ok(crate::cli::screen::devices_value(
            args.flag("probe")?,
        )?)),
        "probe" => Ok(response::ok(crate::cli::screen::probe_value(
            &args.string("device")?,
        )?)),
        "windows" => {
            let selector = args.string("device")?;
            let wait = wait(args)?;
            let (target, _) = crate::cli::screen::device_target(&selector)?;
            let list = crate::daemon::screen_windows_within(target, Some(wait), |_| {})?;
            Ok(response::ok(crate::cli::screen::windows_value(&list)))
        }
        "snapshot" => snapshot(args),
        "families" => {
            let (target, _) = crate::cli::screen::device_target(&args.string("device")?)?;
            // The device feed first (design 5.2); the device is asked only
            // when the background service holds no listing of it.
            let fed = crate::daemon::share_worker_snapshot()
                .ok()
                .and_then(|snapshot| {
                    crate::cli::screen::families::feed_listing(&snapshot, &target)
                });
            let list = match fed {
                Some(list) => list,
                None => crate::daemon::screen_families(target, Some(wait(args)?), |_| {})?,
            };
            Ok(response::ok(
                serde_json::to_value(list).map_err(|error| error.to_string())?,
            ))
        }
        other => Err(format!(
            "unknown action `{other}`: use devices, probe, windows, families or snapshot"
        )),
    }
}

#[cfg(test)]
#[path = "screen_tool_tests.rs"]
mod tests;
