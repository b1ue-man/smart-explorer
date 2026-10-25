//! P5: catalog-bound GUI launch, separate from Exec/Terminal accounts.
//! The network adapter must authorize E4 inside `AppHost::launch`; its returned
//! guard is held through the final platform dispatch. No wire type carries a
//! command, executable path, arguments, environment or user impersonation token.
pub(crate) mod core;
pub(crate) mod os;

pub(crate) use core::model::{
    AppId, AppKind, CatalogApp, CatalogSnapshot, DeviceId, HostSession, LaunchDenial,
    LaunchFacts, LaunchReceipt, LaunchRequest, LaunchState, RequestId, SessionId,
    UserId, WindowBinding,
};
pub(crate) use os::host::{AppHost,CatalogIcons,LaunchAdmission};
pub(crate) use core::grant::{LocalAppGrantChange,LocalAppGrantSnapshot};
