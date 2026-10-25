use std::io;
use crate::apps::core::model::{CatalogApp, HostSession, LaunchRequest, LaunchState};
use crate::screen::core::app_icon::IconReply;

pub(crate) struct InstalledApp<T> {
    pub app: CatalogApp,
    pub fingerprint: String,
    pub target: T,
}
pub(crate) struct PlatformCatalog<T> {
    pub session: HostSession,
    pub apps: Vec<InstalledApp<T>>,
    pub limits: Vec<String>,
}
pub(crate) struct DispatchReceipt {
    pub state: LaunchState,
    pub reason: String,
    pub pid: Option<u32>,
}
pub(crate) trait HostPlatform {
    type Target;
    /// Must obtain fresh OS facts, never a stale caller-supplied session/catalog.
    fn snapshot(&self) -> io::Result<PlatformCatalog<Self::Target>>;
    fn session(&self)->io::Result<HostSession> {self.snapshot().map(|catalog|catalog.session)}
    /// Recheck expected OS login incarnation immediately before dispatch. All
    /// executable data is from `snapshot`; the request supplies identities only.
    fn dispatch(&self, target: &Self::Target, request: &LaunchRequest,
        expected: &HostSession,expires_at_unix_ms:Option<i64>,dispatch_id:&str) -> io::Result<DispatchReceipt>;
    fn icon(&self,_app:&CatalogApp)->io::Result<Option<IconReply>> {Ok(None)}
}
/// The grant deadline is authoritative E4 output, never remote launch input.
pub(crate) fn expired(deadline:Option<i64>)->bool {
    let Some(deadline)=deadline else {return false;};
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_or(true,|now|now.as_millis()>=deadline.max(0) as u128)
}
