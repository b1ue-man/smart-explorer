//! Desktop, CLI, MCP and Android use the same protected embedded-worker IPC.
use std::{net::TcpStream,time::{Duration,Instant}};
use super::ipc_protocol::{read_response,set_stream_timeout,write_request,IpcRequest,IpcResponse,apps::{AppsIpcRequest,AppsIpcReply}};
use crate::{apps::{CatalogSnapshot,LaunchRequest,LaunchReceipt},share::{PeerOpenTarget,apps_wire::{AppsCommand,AppsResult}}};

pub(crate) fn apps_command(target:PeerOpenTarget,command:AppsCommand,wait:Option<Duration>,mut pending:impl FnMut())->Result<(String,String,AppsResult),String> {
    crate::daemon::ensure_worker_ready()?;
    let address=super::ipc_storage::read_ipc_addr().ok_or_else(||"Background worker IPC is not ready".to_string())?;
    let token=super::ipc_storage::read_token().map_err(|e|e.to_string())?;
    let mut stream=TcpStream::connect_timeout(&address,Duration::from_secs(2)).map_err(|e|e.to_string())?;
    let deadline=wait.and_then(|duration|Instant::now().checked_add(duration));
    if wait.is_some()&&deadline.is_none() {return Err("Device-command wait is too large".into());}
    set_stream_timeout(&stream,wait);
    write_request(&mut stream,&IpcRequest::Apps {token,req:AppsIpcRequest::Command {target,command}}).map_err(|e|e.to_string())?;
    loop {
        if let Some(deadline)=deadline {let remaining=deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {return Err("Device-command wait expired; query the same launch request ID instead of starting again".into());}
            set_stream_timeout(&stream,Some(remaining));}
        match read_response(&mut stream).map_err(|e|format!("Device-command daemon request: {e}; an admitted launch may already have started"))? {
            IpcResponse::Apps(AppsIpcReply::Pending)=>pending(),
            IpcResponse::Apps(AppsIpcReply::Completed {node_id,device_id,result})=>return Ok((node_id,device_id,result)),
            IpcResponse::Err {msg,code}=>return Err(code.map_or(msg.clone(),|code|format!("{code}: {msg}"))),
            _=>return Err("Unexpected daemon device-command response".into()),
        }
    }
}
pub(crate) fn apps_catalog(target:PeerOpenTarget,wait:Option<Duration>,pending:impl FnMut())->Result<CatalogSnapshot,String> {
    match apps_command(target,AppsCommand::Catalog,wait,pending)?.2 {AppsResult::Catalog {catalog}=>Ok(catalog),_=>Err("Unexpected app catalog reply".into())}
}
pub(crate) fn apps_icon(target:PeerOpenTarget,app_id:crate::apps::AppId,icon:crate::screen::core::app_icon::IconRef,
    catalog_revision:String,wait:Option<Duration>)->Result<Option<crate::screen::core::app_icon::IconReply>,String> {
    icon.validate().map_err(|e|e.to_string())?;
    match apps_command(target,AppsCommand::Icon {app_id,icon:icon.clone(),catalog_revision},wait,||{})?.2 {
        AppsResult::Icon {reply}=>{if let Some(reply)=&reply {reply.bytes_for(&icon).map_err(|e|e.to_string())?;}Ok(reply)},
        _=>Err("Unexpected app icon reply".into()),
    }
}
pub(crate) fn apps_launch(target:PeerOpenTarget,request:LaunchRequest,wait:Option<Duration>,pending:impl FnMut())->Result<LaunchReceipt,String> {
    match apps_command(target,AppsCommand::Launch {request:request.clone()},wait,pending)?.2 {AppsResult::Launch {receipt} if receipt.request==request=>Ok(receipt),_=>Err("Unexpected app launch reply".into())}
}
pub(crate) fn apps_receipt(target:PeerOpenTarget,request:LaunchRequest,wait:Option<Duration>)->Result<Option<LaunchReceipt>,String> {
    match apps_command(target,AppsCommand::Receipt {request:request.clone()},wait,||{})?.2 {
        AppsResult::Receipt {receipt} if receipt.as_ref().is_none_or(|r|r.request==request)=>Ok(receipt),_=>Err("Unexpected app receipt reply".into())}
}
/// Only authenticated daemon IPC may reconstruct this non-deserializable type.
/// There is no JSON/file/report import entrypoint in the CLI or mobile domain.
pub(crate) fn apps_rdp_probe(target:PeerOpenTarget,wait:Option<Duration>,pending:impl FnMut())->Result<crate::rdp::AuthenticatedProbe,String> {
    let (node,_,result)=apps_command(target,AppsCommand::RdpProbe,wait,pending)?;
    match result {AppsResult::RdpProbe {host}=>crate::rdp::AuthenticatedProbe::from_authenticated_peer(node,host),_=>Err("Unexpected RDP probe reply".into())}
}
fn local(request:AppsIpcRequest)->Result<AppsIpcReply,String> {
    crate::daemon::ensure_worker_ready()?;
    let address=super::ipc_storage::read_ipc_addr().ok_or("Background worker IPC is not ready")?;
    let token=super::ipc_storage::read_token().map_err(|e|e.to_string())?;
    let mut stream=TcpStream::connect_timeout(&address,Duration::from_secs(2)).map_err(|e|e.to_string())?;
    set_stream_timeout(&stream,Some(Duration::from_secs(30)));
    write_request(&mut stream,&IpcRequest::Apps {token,req:request}).map_err(|e|e.to_string())?;
    match read_response(&mut stream).map_err(|e|e.to_string())? {
        IpcResponse::Apps(reply)=>Ok(reply),IpcResponse::Err {msg,..}=>Err(msg),_=>Err("Unexpected local app grant reply".into()),
    }
}
pub(crate) fn apps_local_grants(device:&str,key:&str)->Result<crate::apps::LocalAppGrantSnapshot,String> {
    match local(AppsIpcRequest::LocalGrantSnapshot {device_id:device.into(),public_key:key.into()})? {
        AppsIpcReply::LocalGrantSnapshot {snapshot} if snapshot.peer_device_id==device&&snapshot.peer_public_key==key=>Ok(snapshot),
        _=>Err("Local app grant inventory identity mismatch".into()),
    }
}
pub(crate) fn apps_set_local_grant(change:crate::apps::LocalAppGrantChange)->Result<crate::daemon::ipc_host::exec_grant_journal::ExecGrantPersistResult,String> {
    let target=change.target.clone();
    match local(AppsIpcRequest::SetLocalGrant {change})? {
        AppsIpcReply::LocalGrantMutation {result} if result.target==target=>Ok(result),_=>Err("Local app grant mutation identity mismatch".into()),
    }
}
