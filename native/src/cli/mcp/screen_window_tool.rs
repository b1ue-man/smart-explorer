//! Opening a local native viewer is an explicit mutating MCP operation.
use super::{args::Args,response};
use rmcp::model::CallToolResult;
use serde_json::{json,Value};
pub(super) const DESCRIPTION:&str="Open a separate native viewer for an exact window:@family selector returned by screen action families. The host checks ScreenView and optionally ScreenControl; this command neither approves prompts nor grants access. Native Close requests normal host window close only with current control and host support; Disconnect leaves the host application running. The tool returns local process metadata, never a live image stream.";
pub(super) fn properties()->Value {json!({
    "device":{"type":"string","description":"Saved device selector or share endpoint"},
    "source":{"type":"string","description":"Exact window:@family selector from the same device's authenticated families listing"},
    "control":{"type":"boolean","default":false}
})}
pub(super) fn open(args:&Args)->Result<CallToolResult,String> {
    let pid=crate::cli::screen::open_family(&args.string("device")?,&args.string("source")?,args.flag("control")?)?;
    Ok(response::ok(json!({"launched":true,"pid":pid,"live_image_stream":false})))
}
