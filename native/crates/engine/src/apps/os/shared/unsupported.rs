use super::shared::platform::{DispatchReceipt, HostPlatform, PlatformCatalog};
use crate::apps::core::model::{HostSession, LaunchRequest};
use std::{io, path::Path};
#[derive(Default)]
pub struct Platform;
fn unavailable() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "GUI application launch is unsupported on this host platform",
    )
}
impl HostPlatform for Platform {
    type Target = ();
    fn snapshot(&self) -> io::Result<PlatformCatalog<()>> {
        Err(unavailable())
    }
    fn dispatch(
        &self,
        _: &(),
        _: &LaunchRequest,
        _: &HostSession,
        _: Option<i64>,
        _: &str,
    ) -> io::Result<DispatchReceipt> {
        Err(unavailable())
    }
}
pub fn prepare_journal(_: &Path) -> io::Result<()> {
    Err(unavailable())
}
pub fn sync_journal(_: &Path) -> io::Result<()> {
    Err(unavailable())
}
pub fn publish_record(_: tempfile::NamedTempFile, _: &Path, _: bool) -> io::Result<bool> {
    Err(unavailable())
}
