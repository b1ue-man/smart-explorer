//! Device apps tool cannot approve host prompts or accept a command line.
use std::time::Duration;
use serde_json::{json,Value};
use rmcp::model::CallToolResult;
use super::{args::Args,response};
use crate::share::apps_wire::AppsCommand;
pub(super) const DESCRIPTION:&str="Read the explicitly authorized app catalog, fetch a bounded catalog/hash-bound PNG icon, launch one catalog app with a stable request ID, recover its peer-owned receipt, or read the pinned target's RDP host probe. local_grants reads this host's catalog and current AppLaunch grant for an exact peer; set_local_grant changes only that local app grant using its catalog and policy revision. No commands, paths, argv, environments, approval answers, login or imported trusted RDP reports. After a timeout query receipt with the same request instead of starting again.";
pub(super) fn properties()->Value {json!({
    "action":{"type":"string","enum":["catalog","icon","launch","receipt","rdp_probe","local_grants","set_local_grant"]},
    "peer_device_id":{"type":"string","description":"local_grants: exact recipient device ID"},
    "peer_public_key":{"type":"string","description":"local_grants: exact recipient public key"},
    "change":{"type":"object","description":"set_local_grant: exact LocalAppGrantChange using the local_grants catalog, relation, and policy revision"},
    "device":{"type":"string","description":"Saved device selector or share endpoint"},
    "app_id":{"type":"string","description":"icon: exact app ID from the authorized catalog"},
    "catalog_revision":{"type":"string","description":"icon: current catalog revision"},
    "icon":{"type":"object","additionalProperties":false,"required":["version","app_id","sha256","width","height"],
        "properties":{"version":{"type":"integer"},"app_id":{"type":"string"},"sha256":{"type":"string"},"width":{"type":"integer"},"height":{"type":"integer"}}},
    "request":{"type":"object","additionalProperties":false,"required":["request_id","target_device_id","app_id","user_id","session_id","catalog_revision"],
        "properties":{"request_id":{"type":"string"},"target_device_id":{"type":"string"},"app_id":{"type":"string"},
            "user_id":{"type":"string"},"session_id":{"type":"string"},"catalog_revision":{"type":"string"}}},
    "wait":{"type":"integer","minimum":1,"default":30}
})}
pub(super) fn apps(args:&Args)->Result<CallToolResult,String> {
    let action=args.string("action")?;
    if action=="local_grants" {
        let snapshot=crate::daemon::apps_local_grants(&args.string("peer_device_id")?,&args.string("peer_public_key")?)?;
        return Ok(response::ok(serde_json::to_value(snapshot).map_err(|error|error.to_string())?));
    }
    if action=="set_local_grant" {
        let change=serde_json::from_value(args.value("change").cloned().ok_or_else(||"change is required".to_string())?).map_err(|error|error.to_string())?;
        return Ok(response::ok(serde_json::to_value(crate::daemon::apps_set_local_grant(change)?).map_err(|error|error.to_string())?));
    }
    let (target,_)=crate::cli::screen::device_target(&args.string("device")?)?;
    let command=match action.as_str() {
        "catalog"=>AppsCommand::Catalog,"rdp_probe"=>AppsCommand::RdpProbe,
        "icon"=>AppsCommand::Icon {app_id:crate::apps::AppId::try_from(args.string("app_id")?).map_err(|e|e.to_string())?,
            icon:serde_json::from_value(args.value("icon").cloned().ok_or_else(||"icon is required".to_string())?).map_err(|e|e.to_string())?,
            catalog_revision:args.string("catalog_revision")?},
        action @ ("launch"|"receipt")=>{
            let request=serde_json::from_value(args.value("request").cloned().ok_or_else(||"request is required".to_string())?).map_err(|e|e.to_string())?;
            if action=="launch" {AppsCommand::Launch {request}} else {AppsCommand::Receipt {request}}
        },
        value=>return Err(format!("Unknown apps action {value}; use catalog, icon, launch, receipt, rdp_probe, local_grants or set_local_grant")),
    };
    let seconds=args.opt_u64("wait")?.unwrap_or(30);if seconds==0 {return Err("wait must be positive".into());}
    let (node,device,result)=crate::daemon::apps_command(target,command,Some(Duration::from_secs(seconds)),||{})?;
    Ok(response::ok(json!({"node_id":node,"device_id":device,"result":result})))
}
