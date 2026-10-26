//! MCP owns its viewer sockets; responses contain metadata, never live pixels.
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
pub(super) const DESCRIPTION: &str = "Read optional login installation, administrative rights/platform limits and owned viewer sessions. Open only with an explicit administrator-provided public machine target and acknowledge_machine_pin; a normal device selector is not machine proof. Every start/capture/input/release/SAS/stop uses the same daemon owner socket and independent peer-signed broker contract. The existing five-second signed-operation idle lease is never renewed by status/challenge. No automatic input retry, service installation, policy mutation, password-specific argument, live picture stream or image bytes in tool results. Capture returns bounded frame metadata only. Close ends the owner and releases inputs.";
pub(super) fn properties() -> Value {
    json!({
        "action":{"type":"string","enum":["status","sessions","session","open","challenge","start","capture","input","release_inputs","secure_attention","close"]},
        "id":{"type":"string","description":"Exact owning login session ID"},
        "target":{"type":"object","additionalProperties":false,
            "required":["format","machine_id","public_key","direct_addresses","relay_urls","use_default_relays","allow_plaintext_relays"],
            "properties":{"format":{"type":"integer","const":1},"machine_id":{"type":"string"},"public_key":{"type":"string"},
                "direct_addresses":{"type":"array","maxItems":32,"items":{"type":"string"}},"relay_urls":{"type":"array","maxItems":16,"items":{"type":"string"}},
                "use_default_relays":{"type":"boolean"},"allow_plaintext_relays":{"type":"boolean"}}},
        "target_user":{"type":"string","description":"Exact administratively granted host user ID; Windows account SID or Linux UID"},
        "acknowledge_machine_pin":{"type":"boolean","description":"Explicit user pinning of this public machine target; never inferred from a normal peer"},
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
            let target = serde_json::from_value(
                args.value("target")
                    .cloned()
                    .ok_or_else(|| "target is required".to_string())?,
            )
            .map_err(|error| error.to_string())?;
            let mut viewer = LoginViewer::open(
                target,
                args.string("target_user")?,
                args.flag("acknowledge_machine_pin")?,
            )?;
            let started = viewer.command(LoginCommand::Start)?;
            let id = viewer.view().session.clone();
            let result =
                json!({"session":viewer.view(),"start":started.reply,"live_image_stream":false});
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
                    let value = json!({"reply":frame.reply,"pixel_bytes":frame.pixels.len(),"live_image_stream":false});
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
