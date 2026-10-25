//! `se apps catalog/launch/receipt/rdp-probe`, always through daemon IPC.
use clap::{Args,Subcommand};
use serde_json::json;
use crate::apps::{LaunchRequest,AppId,DeviceId,UserId,SessionId,RequestId};
#[derive(Args)]
pub(super) struct AppsArgs {#[command(subcommand)] command:AppsCommand}
#[derive(Subcommand)]
enum AppsCommand {
    Catalog {device:String,#[arg(long)] json:bool},
    Icon {device:String,#[arg(long)] app_id:String,#[arg(long)] sha256:String,#[arg(long)] width:u32,
        #[arg(long)] height:u32,#[arg(long,default_value_t=1)] version:u8,#[arg(long)] catalog_revision:String,#[arg(long)] json:bool},
    Launch {device:String,#[arg(long)] app_id:String,#[arg(long)] target_device_id:String,#[arg(long)] user_id:String,
        #[arg(long)] session_id:String,#[arg(long)] catalog_revision:String,#[arg(long)] request_id:String,#[arg(long)] json:bool},
    Receipt {device:String,#[arg(long)] app_id:String,#[arg(long)] target_device_id:String,#[arg(long)] user_id:String,
        #[arg(long)] session_id:String,#[arg(long)] catalog_revision:String,#[arg(long)] request_id:String,#[arg(long)] json:bool},
    RdpProbe {device:String,#[arg(long)] json:bool},
    LocalGrants {#[arg(long)] peer_device_id:String,#[arg(long)] peer_public_key:String,#[arg(long)] json:bool},
    SetLocalGrant {#[arg(long,help="Exact LocalAppGrantChange JSON from the local_grants snapshot")] change:String,#[arg(long)] json:bool},
}
pub(super) fn run(args:AppsArgs)->Result<i32,String> {
    let compact=match &args.command {AppsCommand::Catalog {json,..}|AppsCommand::Icon {json,..}|AppsCommand::Launch {json,..}|
        AppsCommand::Receipt {json,..}|AppsCommand::RdpProbe {json,..}|AppsCommand::LocalGrants {json,..}|AppsCommand::SetLocalGrant {json,..}=>*json};
    let value=match args.command {
        AppsCommand::LocalGrants {peer_device_id,peer_public_key,..}=>{
            serde_json::to_value(crate::daemon::apps_local_grants(&peer_device_id,&peer_public_key)?).map_err(|error|error.to_string())?
        },
        AppsCommand::SetLocalGrant {change,..}=>{
            let change=serde_json::from_str(&change).map_err(|error|format!("Invalid LocalAppGrantChange: {error}"))?;
            serde_json::to_value(crate::daemon::apps_set_local_grant(change)?).map_err(|error|error.to_string())?
        },
        AppsCommand::Catalog {device,json:_}=>{
            let (target,_)=super::screen::device_target(&device)?;
            json!({"catalog":crate::daemon::apps_catalog(target,None,||eprintln!("Warte auf Gerätefreigabe …"))?})
        },
        AppsCommand::Icon {device,app_id,sha256,width,height,version,catalog_revision,json:_}=>{
            let (target,_)=super::screen::device_target(&device)?;
            let id=AppId::try_from(app_id.clone()).map_err(|e|e.to_string())?;
            let icon=crate::screen::core::app_icon::IconRef {version,app_id,sha256,width,height};
            json!({"icon":crate::daemon::apps_icon(target,id,icon,catalog_revision,Some(std::time::Duration::from_secs(30)))?})
        },
        AppsCommand::Launch {device,app_id,target_device_id,user_id,session_id,catalog_revision,request_id,json:_}=>{
            let (target,_)=super::screen::device_target(&device)?;
            let request=request(app_id,target_device_id,user_id,session_id,catalog_revision,request_id)?;
            json!({"receipt":crate::daemon::apps_launch(target,request,None,||eprintln!("Warte auf Programmstartfreigabe …"))?})
        },
        AppsCommand::Receipt {device,app_id,target_device_id,user_id,session_id,catalog_revision,request_id,json:_}=>{
            let (target,_)=super::screen::device_target(&device)?;
            let request=request(app_id,target_device_id,user_id,session_id,catalog_revision,request_id)?;
            json!({"receipt":crate::daemon::apps_receipt(target,request,Some(std::time::Duration::from_secs(30)))?})
        },
        AppsCommand::RdpProbe {device,json:_}=>{
            let (target,_)=super::screen::device_target(&device)?;
            let (_,_,result)=crate::daemon::apps_command(target,crate::share::apps_wire::AppsCommand::RdpProbe,None,||{})?;
            serde_json::to_value(result).map_err(|e|e.to_string())?
        },
    };
    if compact {println!("{value}");} else {println!("{}",serde_json::to_string_pretty(&value).map_err(|e|e.to_string())?);}
    Ok(0)
}
pub(super) fn request(app:String,target:String,user:String,session:String,revision:String,id:String)->Result<LaunchRequest,String> {
    Ok(LaunchRequest {app_id:AppId::try_from(app).map_err(|e|e.to_string())?,target_device_id:DeviceId::try_from(target).map_err(|e|e.to_string())?,
        user_id:UserId::try_from(user).map_err(|e|e.to_string())?,session_id:SessionId::try_from(session).map_err(|e|e.to_string())?,
        catalog_revision:revision,request_id:RequestId::try_from(id).map_err(|e|e.to_string())?})
}
