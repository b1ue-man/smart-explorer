//! MCP owns its viewer sockets; responses contain metadata, never live pixels.
//! `open` takes a paired `device` (E22) or an explicit administrative `target`.
use super::{args::Args, response};
use crate::daemon::{LoginClose, LoginCommand, LoginViewer};
use rmcp::model::CallToolResult;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
struct Owned {
    viewer: Mutex<LoginViewer>,
    close: LoginClose,
}
static OWNED: Mutex<BTreeMap<String, Arc<Owned>>> = Mutex::new(BTreeMap::new());
pub(super) const DESCRIPTION: &str = "Opens and drives the login screen (sign-in, lock screen, Windows secure desktop) of a paired device and reads the optional login installation and owned viewer sessions. Open with `device` (device ID, contact ID or name): the device sends its machine target over the pinned Share connection once it allows this device „screen control“, so no file and no pin acknowledgement are needed; `target_user` is optional there. The administrative form with an explicit public `target`, `target_user` and `acknowledge_machine_pin` stays. Every start/capture/input/release/SAS/stop uses the same daemon owner socket and independent peer-signed broker contract. The existing five-second signed-operation idle lease is never renewed by status/challenge. No automatic input retry, service installation, policy mutation, password-specific argument, live picture stream or image bytes in tool results. Capture returns bounded frame metadata only. Close ends the owner and releases inputs.";
pub(super) fn properties() -> Value {
    json!({
        "action":{"type":"string","enum":["status","sessions","session","open","challenge","start","capture","input","release_inputs","secure_attention","close"]},
        "id":{"type":"string","description":"Exact owning login session ID"},
        "device":{"type":"string","description":"Paired device (device ID, contact ID or name) whose login screen to open; it sent its machine target over the pinned Share connection"},
        "target":{"type":"object","additionalProperties":false,
            "required":["format","machine_id","public_key","direct_addresses","relay_urls","use_default_relays","allow_plaintext_relays"],
            "properties":{"format":{"type":"integer","const":1},"machine_id":{"type":"string"},"public_key":{"type":"string"},
                "direct_addresses":{"type":"array","maxItems":32,"items":{"type":"string"}},"relay_urls":{"type":"array","maxItems":16,"items":{"type":"string"}},
                "use_default_relays":{"type":"boolean"},"allow_plaintext_relays":{"type":"boolean"}}},
        "target_user":{"type":"string","description":"Host account (Windows SID, Linux UID); required with `target`, optional with `device` (the device names it)"},
        "acknowledge_machine_pin":{"type":"boolean","description":"Explicit user pinning of an administrator-provided `target`; not used with `device`"},
        "frame_id":{"type":"integer","minimum":0},
        "events":{"type":"array","minItems":1,"maxItems":crate::screen::core::input_model::MAX_INPUT_EVENTS,
            "items":{"type":"object","description":"A typed screen InputEvent; Android actions are unavailable at login"}}
    })
}
pub(super) fn login(args: &Args) -> Result<CallToolResult, String> {
    let action = args.string("action")?;
    let value = match action.as_str() {
        "status" => json!({"installation":crate::daemon::login_status()?}),
        "sessions" => json!({"sessions":crate::daemon::login_sessions()?}),
        "session" => json!({"session":crate::daemon::login_session(args.string("id")?)?}),
        "open" => {
            purge()?;
            if OWNED
                .lock()
                .map_err(|_| "MCP login owner registry is unavailable")?
                .len()
                >= 64
            {
                return Err("MCP login session admission is full".into());
            }
            let device = args.value("device").and_then(Value::as_str);
            let (mut viewer, paired) = match (device, args.value("target")) {
                (Some(_), Some(_)) => return Err("give either device or target".into()),
                (Some(device), None) => {
                    let paired = crate::share::login_transport::paired_login_target(device)
                        .map_err(|error| error.to_string())?;
                    let user = args
                        .value("target_user")
                        .and_then(Value::as_str)
                        .filter(|user| !user.is_empty())
                        .map_or_else(|| paired.target_user.clone(), str::to_string);
                    let viewer = LoginViewer::open(paired.target.clone(), user, true)?;
                    let summary = json!({"device_id":paired.device_id,"device_name":paired.device_name,
                        "machine_id":paired.target.machine_id,"machine_public_key":paired.target.public_key,
                        "target_user":paired.target_user,"received_at":paired.received_at});
                    (viewer, summary)
                }
                (None, Some(target)) => {
                    let target = serde_json::from_value(target.clone())
                        .map_err(|error| error.to_string())?;
                    let viewer = LoginViewer::open(
                        target,
                        args.string("target_user")?,
                        args.flag("acknowledge_machine_pin")?,
                    )?;
                    (viewer, Value::Null)
                }
                (None, None) => return Err("device or target is required".into()),
            };
            let started = viewer.command(LoginCommand::Start)?;
            let id = viewer.view().session.clone();
            let result = json!({"session":viewer.view(),"start":started.reply,"paired":paired,"live_image_stream":false});
            if viewer.view().challenge.is_some() {
                let owned = Arc::new(Owned {
                    close: viewer.control(),
                    viewer: Mutex::new(viewer),
                });
                OWNED
                    .lock()
                    .map_err(|_| "MCP login owner registry is unavailable")?
                    .insert(id, owned);
            }
            result
        }
        "close" => {
            let id = args.string("id")?;
            if let Some(owned) = OWNED
                .lock()
                .map_err(|_| "MCP login owner registry is unavailable")?
                .remove(&id)
            {
                owned.close.close();
            }
            json!({"stopped":crate::daemon::login_stop(Some(id))?})
        }
        "challenge" | "start" | "capture" | "input" | "release_inputs" | "secure_attention" => {
            let id = args.string("id")?;
            let owned = OWNED
                .lock()
                .map_err(|_| "MCP login owner registry is unavailable")?
                .get(&id)
                .cloned()
                .ok_or_else(|| "this MCP process does not own that login session".to_string())?;
            let command = match action.as_str() {
                "challenge" => LoginCommand::Challenge,
                "start" => LoginCommand::Start,
                "capture" => LoginCommand::Capture {
                    frame_id: args
                        .opt_u64("frame_id")?
                        .ok_or_else(|| "frame_id is required".to_string())?,
                },
                "input" => LoginCommand::Input {
                    events: serde_json::from_value(
                        args.value("events")
                            .cloned()
                            .ok_or_else(|| "events are required".to_string())?,
                    )
                    .map_err(|error| error.to_string())?,
                },
                "release_inputs" => LoginCommand::ReleaseInputs,
                _ => LoginCommand::SecureAttention,
            };
            let result = owned
                .viewer
                .lock()
                .map_err(|_| "MCP login session is unavailable")?
                .command(command);
            match result {
                Ok(frame) => {
                    let terminal = frame.reply.view.challenge.is_none();
                    let pixel_bytes = frame
                        .image
                        .as_ref()
                        .map_or(0, |image| image.image().data().len());
                    let value = json!({"reply":frame.reply,"pixel_bytes":pixel_bytes,"live_image_stream":false});
                    if terminal {
                        remove(&id)?;
                    }
                    value
                }
                Err(error) => {
                    remove(&id)?;
                    return Err(error);
                }
            }
        }
        _ => return Err(format!("unknown login action {action}")),
    };
    Ok(response::ok(value))
}
fn remove(id: &str) -> Result<(), String> {
    if let Some(owned) = OWNED
        .lock()
        .map_err(|_| "MCP login owner registry is unavailable")?
        .remove(id)
    {
        owned.close.close();
    }
    Ok(())
}
fn purge() -> Result<(), String> {
    let live = crate::daemon::login_sessions()?;
    let mut sessions = OWNED
        .lock()
        .map_err(|_| "MCP login owner registry is unavailable")?;
    sessions.retain(|id, owned| {
        let keep = live.iter().any(|session| &session.session == id);
        if !keep {
            owned.close.close();
        }
        keep
    });
    Ok(())
}
/// Main invokes this after pending tool calls finish and before MCP runtime exit.
/// It interrupts each owner immediately, then waits for the daemon's real drain.
pub(super) fn shutdown() -> Result<usize, String> {
    let sessions = OWNED
        .lock()
        .map_err(|_| "MCP login owner registry is unavailable")?
        .clone();
    for owned in sessions.values() {
        owned.close.close();
    }
    for id in sessions.keys() {
        crate::daemon::login_stop(Some(id.clone()))?;
    }
    let mut remaining = OWNED
        .lock()
        .map_err(|_| "MCP login owner registry is unavailable")?;
    for id in sessions.keys() {
        remaining.remove(id);
    }
    Ok(sessions.len())
}
