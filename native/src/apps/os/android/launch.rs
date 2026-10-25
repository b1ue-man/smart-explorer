use std::io;
use crate::apps::core::model::{CatalogApp,HostSession,LaunchRequest};
use crate::screen::core::app_icon::IconReply;
use super::super::shared::platform::{DispatchReceipt,HostPlatform,InstalledApp,PlatformCatalog};
#[derive(Default)]
pub(crate) struct Platform;
pub(crate) struct Target {fingerprint:String}
impl HostPlatform for Platform {
    type Target=Target;
    fn snapshot(&self)->io::Result<PlatformCatalog<Target>> {
        let catalog=super::bridge::catalog()?;
        let apps=catalog.apps.into_iter().map(|entry|InstalledApp {app:entry.app,
            fingerprint:entry.fingerprint.clone(),target:Target {fingerprint:entry.fingerprint}}).collect();
        Ok(PlatformCatalog {session:catalog.session,apps,limits:catalog.limits})
    }
    fn dispatch(&self,target:&Target,request:&LaunchRequest,expected:&HostSession,deadline:Option<i64>,dispatch_id:&str)->io::Result<DispatchReceipt> {
        super::bridge::dispatch(request,expected,&target.fingerprint,deadline,dispatch_id)
    }
    fn icon(&self,app:&CatalogApp)->io::Result<Option<IconReply>> {
        match &app.icon {Some(icon)=>super::bridge::icon(&app.app_id,icon),None=>Ok(None)}
    }
}
