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
//!
//! `transport` (user 2026-11-25) reads or sets the desktop viewer's
//! „Übertragung“ (`screen_delivery.rs`) and answers the rate the host
//! measured for each screen session this device shows to another one.
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

pub(super) const DESCRIPTION: &str = "Looks at the screen of a paired device: `devices` lists the devices and their screen rights, `probe` shows platform, monitors, codecs, limits and `virtual_display` (whether the device can show its desktop on se's own display in the viewer's resolution; `se screen view --virtual-display auto|off|request --display-size WxH` chooses it; with `live: true` the viewer's „Monitor“ and „Auflösung“ change it in the running session; `text` is the German line the viewers show; `inactive`/`setup_needed`: the first viewer with „screen control“ that asks installs the display driver, on Windows with one administrator confirmation there unless se runs privileged; while it installs, sessions report `setup` (`windows_display_driver_install_prompt` or `_installing`), keep the current monitor and switch after success; `unsupported` with `windows_display_driver_declined` or `windows_display_driver_install_failed: <message>` names the cause, a new request tries again), `windows` lists its windows, `families` returns confirmed window-family selectors, `snapshot` returns one still picture (PNG, long edge at most 1568 px unless `max_edge` says otherwise), `audio` reads or sets („on“) whether the desktop viewer plays the device's sound („Ton“, default on; also `se screen view --audio on|off`), `transport` reads or sets its „Übertragung“ (`udp`: the pictures go as UDP datagrams directly between the devices, an incomplete picture is dropped, default; `tcp`: over a direct TCP connection, every picture arrives, later on a slow line; also `se screen view --transport`) and lists the measured rate of each session this device hosts. `probe` also answers `viewer_codecs`: every „Codec“ of the viewer (`auto`, `h264`, `hevc`, `av1`, `sharp`, `lossless`) with `usable` and, when this pair cannot run it, the German `reason` (`se screen view --codec` refuses those). The other device must have granted „screen view“; if it asks its user, the call waits up to `wait` seconds (default 30) and then fails with `approval_pending`. There is no live video and no way to answer a confirmation here; input goes through the tool `screen_input`.";

const DEVICE: &str = "Device: a `share://direct/<id>` or `share://room/<room>/<device>` endpoint, a name, an id or a fingerprint prefix; `devices` lists them.";

pub(super) fn properties() -> Value {
    json!({
        "action": {
            "type": "string",
            "enum": ["devices", "probe", "windows", "families", "snapshot", "audio", "transport"],
            "description": "devices, probe, windows, confirmed families, snapshot, audio (the viewer's „Ton“) or transport (its „Übertragung“)."
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
        },
        "on": {
            "type": "boolean",
            "description": "audio: play the device's sound in the desktop viewer (true) or not (false); omitted reads the setting."
        },
        "transport": {
            "type": "string",
            "enum": ["udp", "tcp", "realtime", "reliable"],
            "description": "transport: the desktop viewer's „Übertragung“, `udp` (UDP datagrams directly between the devices, an incomplete picture is given up; the default) or `tcp` (a direct TCP connection, every picture arrives, later on a slow line); `realtime`/`reliable` are the stored keys of the same two; omitted reads the setting."
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

/// `audio` (plan 2026-11-17 P9): the desktop viewer's „Ton“, read or set.
fn audio(args: &Args) -> Result<CallToolResult, String> {
    match args.value("on") {
        None => {}
        Some(Value::Bool(on)) => crate::app::screen_audio::set_audio(*on)
            .map_err(|error| format!("cannot store the audio choice: {error}"))?,
        Some(_) => return Err("argument `on` must be true or false".into()),
    }
    Ok(response::ok(
        json!({"audio": crate::app::screen_audio::audio_on()}),
    ))
}

/// `transport` (user 2026-11-25): the desktop viewer's „Übertragung“, read
/// or set (a value other than `udp`, `tcp`, `realtime` or `reliable` is refused before
/// anything is stored), with the rate the host measured for each screen
/// session this device shows (`hosted_sessions`, as `se screen sessions`
/// names it; `hosted_sessions_error` when the background service does not
/// answer).
fn transport(args: &Args) -> Result<CallToolResult, String> {
    use crate::app::screen_delivery::{self as delivery, Delivery};
    if let Some(text) = args.opt_string("transport")? {
        let mode = Delivery::parse(&text).ok_or("argument `transport` must be udp or tcp")?;
        delivery::set_delivery(mode)
            .map_err(|error| format!("cannot store the transport choice: {error}"))?;
    }
    let mode = delivery::delivery();
    let mut answer = json!({"transport": mode.key(), "label": mode.label(), "hint": mode.hint()});
    match crate::daemon::screen_sessions() {
        Ok((_, sessions)) => answer["hosted_sessions"] = hosted_rates(&sessions),
        Err(error) => answer["hosted_sessions_error"] = json!(error.to_string()),
    }
    Ok(response::ok(answer))
}

/// Each session this device hosts with the „Übertragung“ it runs and the
/// rate it measured (`null` from a host that reports none).
fn hosted_rates(sessions: &[crate::screen::core::registry::SessionView]) -> Value {
    sessions
        .iter()
        .map(|session| {
            let chain = session.chain.as_ref();
            json!({
                "session": session.id,
                "device": session.device_name,
                "delivery": chain.and_then(|chain| chain.delivery).map(|mode| mode.key()),
                "measured_kbps": chain.and_then(|chain| chain.measured_kbps),
                "measured_age_ms": chain.and_then(|chain| chain.measured_age_ms),
            })
        })
        .collect()
}

/// Each „Codec“ of the desktop viewer with this device (user rule
/// 2026-11-24): `{codec, label, usable, reason}` from the device's `codecs`
/// and this device's decoders under the stored „Dekodierer“ and
/// „Dekodier-Bibliothek“ (`codec_offer::codec_verdict`); `null` when the
/// device shows nothing to this one.
fn viewer_codecs(probe: &Value) -> Value {
    use crate::app::screen_prefs::codec_offer::{codec_verdict, Viewer};
    use crate::screen::core::negotiate::{CodecOffer, CodecSetting};
    let shown = probe["supported"] == Value::Bool(true) && probe["view"] != "off";
    let host = serde_json::from_value::<Vec<CodecOffer>>(probe["codecs"].clone());
    let Some(host) = host.ok().filter(|_| shown) else {
        return Value::Null;
    };
    let decoder = crate::app::screen_prefs::decoder_choice();
    let offers = |library| {
        crate::screen::core::negotiate::set_decoder_preference(decoder);
        crate::screen::core::decoder_library::set_decoder_library(library);
        crate::screen::os::codec_inventory::registry().decoder_offers()
    };
    // A chosen library without decoders here runs as „Automatisch“ in the
    // viewer (`codec_offer::effective_library`).
    let mut library = crate::app::screen_prefs::stored_output().0;
    let mut decoders = offers(library);
    if !decoders.iter().any(|offer| offer.codec.is_video()) {
        library = crate::screen::core::decoder_library::DecoderLibrary::Automatic;
        decoders = offers(library);
    }
    let viewer = Viewer {
        decoders: &decoders,
        decoder,
        library,
    };
    CodecSetting::ALL
        .iter()
        .map(|codec| {
            let verdict = codec_verdict(*codec, Some(&host), &viewer);
            let reason = verdict.reason().map(str::to_string);
            json!({
                "codec": codec.key(),
                "label": codec.label(),
                "usable": reason.is_none(),
                "reason": reason,
            })
        })
        .collect()
}

pub(super) const INPUT_DESCRIPTION: &str = "Clicks, moves the pointer, scrolls, types text, presses a key or a key combination, or runs an Android system action on the screen of a paired device (one input per call). The other device must have granted „screen control“ to this device; if it asks its user, the call waits up to `wait` seconds (default 30) and then fails with `approval_pending`. Coordinates are pixels of the source (in its own size; a host that does not name a window's size takes percentages only) or percentages (`50%`). The answer says whether the input was sent (`sent`) and whether the device confirmed that it applied it (`applied`: `true` or `false` from a device that sends input receipts, which answers once the input is settled, on an Android device once it performed or refused it; `null` from an older device, whose answer is awaited 1.5 s, 3.8 s on an Android device, so `sent` is unconfirmed, and `null` with `unconfirmed` (the reason) when a device with receipts sent none within the computed limit), why the device blocked it (`blocked`: `elevated_window`, `secure_desktop`, `control_unavailable`, `unsupported_on_host`) or why it held the input back without applying it (`held`: `viewer_hidden`, `waiting_for_picture`, `release_pending`, `backlog_full`, `other`; `waiting_for_picture` also when no picture came within five seconds, the input is then not sent) or why it took the input but did not perform it (`refused`: the device's sentence, e.g. a mouse button without counterpart or a password field), or that the text was not typed to its end (`stopped`: `untyped` characters, `exact`, the German `text`, and `block` or `held` when a block or hold stopped it partway; without them the control went or the session ended). Ctrl+Alt+Delete is never sent.";

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

/// The tool `screen_input`: one input, the answer
/// `{"device","sent","applied","unconfirmed","blocked","held","refused","stopped"}`.
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
    let outcome = crate::cli::screen::input::deliver(target, source, &input, Some(wait), |_| {})?;
    let mut answer = crate::cli::screen::input::outcome_value(&outcome);
    answer["device"] = json!(device);
    Ok(response::ok(answer))
}

pub(super) fn screen(args: &Args) -> Result<CallToolResult, String> {
    match args.string("action")?.as_str() {
        "devices" => Ok(response::ok(crate::cli::screen::devices_value(
            args.flag("probe")?,
        )?)),
        "probe" => {
            let mut probe = crate::cli::screen::probe_value(&args.string("device")?)?;
            probe["viewer_codecs"] = viewer_codecs(&probe);
            Ok(response::ok(probe))
        }
        "windows" => {
            let selector = args.string("device")?;
            let wait = wait(args)?;
            let (target, _) = crate::cli::screen::device_target(&selector)?;
            let list = crate::daemon::screen_windows_within(target, Some(wait), |_| {})?;
            Ok(response::ok(crate::cli::screen::windows_value(&list)))
        }
        "snapshot" => snapshot(args),
        "audio" => audio(args),
        "transport" => transport(args),
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
            "unknown action `{other}`: use devices, probe, windows, families, snapshot, audio or transport"
        )),
    }
}

#[cfg(test)]
#[path = "screen_tool_tests.rs"]
mod tests;
