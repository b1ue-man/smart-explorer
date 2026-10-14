//! `sessions`: this server's persistent shells and the attachable sessions
//! of the user's other `se` processes (`se shell -T --attachable`, other
//! agents). `list` shows them with their kind, `attach` joins a command
//! session (a terminal session of `se shell` is refused), `activity` returns
//! what the other participants did since the last call, `close` ends an own
//! session or detaches from an attached one.
use super::args::Args;
use super::response;
use super::session::Session;
use crate::cli::shell_session::SessionKind;
use rmcp::model::CallToolResult;
use serde_json::json;

pub(super) fn sessions(session: &Session, args: &Args<'_>) -> Result<CallToolResult, String> {
    let action = args
        .opt_string("action")?
        .unwrap_or_else(|| "list".to_string());
    match action.as_str() {
        "list" => Ok(response::ok(json!({ "sessions": session.shells.list() }))),
        "attach" => {
            let port = session.shells.attach(&args.string("session")?)?;
            let info = port.client.info();
            Ok(response::ok(json!({
                "session": info.name,
                "id": info.id,
                "kind": SessionKind::Commands.label(),
                "role": "attached",
                "target": info.target,
                "dialect": info.dialect,
                "cwd": info.cwd,
                "owner": info.owner,
                "next": format!(
                    "run commands with exec and session \"{}\"; sessions with action activity shows what the others did there",
                    info.name
                ),
            })))
        }
        "close" => {
            let name = args.string("session")?;
            let owned = session.shells.close(&name)?;
            Ok(response::ok(json!({
                "session": name,
                "closed": owned,
                "detached": !owned,
            })))
        }
        "activity" => Ok(response::ok(
            session.shells.activity(&args.string("session")?)?,
        )),
        other => Err(format!(
            "unknown action `{other}`; use list, attach, close or activity"
        )),
    }
}
