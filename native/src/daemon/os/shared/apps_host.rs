//! Install the app host only with the worker's real, pinned device identity.
use crate::apps::{CatalogApp, CatalogIcons, DeviceId};
use crate::screen::core::app_icon::{IconRef, IconReply};

pub(super) fn configure(service: &crate::share::ShareService) -> Result<(), String> {
    let device = DeviceId::try_from(service.identity.device_id.clone()).map_err(|e| e.to_string())?;
    let root = crate::support_dirs::app_data_dir().join("app-launch");
    service.configure_apps_host(device, root, icons()).map_err(|e| format!("App host: {e}"))
}

#[cfg(any(windows, target_os = "linux"))]
fn icons() -> Option<CatalogIcons> {
    Some(CatalogIcons { references, content })
}
#[cfg(not(any(windows, target_os = "linux")))]
fn icons() -> Option<CatalogIcons> { None }

#[cfg(any(windows, target_os = "linux"))]
fn references(apps: &mut [CatalogApp]) {
    for app in apps {
        app.icon = crate::screen::os::app_icons::catalog_icon(app.app_id.as_str()).map(|reply| reply.icon);
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn content(app: &CatalogApp, expected: &IconRef) -> std::io::Result<Option<IconReply>> {
    let reply = crate::screen::os::app_icons::catalog_icon(app.app_id.as_str());
    if let Some(reply) = &reply { reply.bytes_for(expected)?; }
    Ok(reply)
}
