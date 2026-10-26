//! AppsFolder is the installed Start-menu/Store application namespace.
//! Keep its real shell identity; never accept a caller's executable or verb.
use super::super::shared::platform::InstalledApp;
use crate::apps::core::model::{AppId, AppKind, CatalogApp};
use sha2::{Digest, Sha256};
use std::{fs, io};
use windows::{
    core::{w, Interface, PCWSTR},
    Win32::{
        System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED},
        UI::Shell::{PropertiesSystem::PSGetPropertyKeyFromName, *},
    },
};

const MAX_APPS: usize = 8192;
pub(crate) struct Target {
    pub(super) parsing_name: String,
    pub(super) fingerprint: String,
}
pub(super) fn error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("Windows app catalog: {error}"))
}
pub(super) struct Apartment;
impl Apartment {
    pub(super) fn enter() -> io::Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(error)?;
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
pub(super) fn text(item: &IShellItem, kind: SIGDN) -> io::Result<String> {
    let value = unsafe { item.GetDisplayName(kind) }.map_err(error)?;
    let result = unsafe { value.to_string() }.map_err(error);
    unsafe {
        CoTaskMemFree(Some(value.0.cast()));
    }
    result
}
fn property(item: &IShellItem, name: &str) -> Option<String> {
    let value: IShellItem2 = item.cast().ok()?;
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut key = PropertiesSystem::PROPERTYKEY::default();
    unsafe { PSGetPropertyKeyFromName(PCWSTR(name.as_ptr()), &mut key) }.ok()?;
    let text = unsafe { value.GetString(&key) }.ok()?;
    let result = unsafe { text.to_string() }.ok();
    unsafe {
        CoTaskMemFree(Some(text.0.cast()));
    }
    result
}
pub(super) fn fingerprint(item: &IShellItem) -> io::Result<String> {
    let mut hash = Sha256::new();
    hash.update(text(item, SIGDN_DESKTOPABSOLUTEPARSING)?.as_bytes());
    for name in [
        "System.AppUserModel.ID",
        "System.AppUserModel.PackageFullName",
        "System.AppUserModel.PackageInstallPath",
        "System.Link.TargetParsingPath",
        "System.Link.Arguments",
        "System.ItemPathDisplay",
    ] {
        if let Some(value) = property(item, name) {
            hash.update(name.as_bytes());
            hash.update((value.len() as u64).to_le_bytes());
            hash.update(value.as_bytes());
            if let Ok(meta) = fs::metadata(&value) {
                use std::os::windows::fs::MetadataExt;
                hash.update(meta.file_size().to_le_bytes());
                hash.update(meta.last_write_time().to_le_bytes());
            }
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub(super) fn list() -> io::Result<Vec<InstalledApp<Target>>> {
    let _apartment = Apartment::enter()?;
    let folder: IShellItem =
        unsafe { SHCreateItemFromParsingName(w!("shell:AppsFolder"), None) }.map_err(error)?;
    let entries: IEnumShellItems =
        unsafe { folder.BindToHandler(None, &BHID_EnumItems) }.map_err(error)?;
    let mut result = Vec::new();
    loop {
        let mut row = [None];
        let mut count = 0;
        unsafe { entries.Next(&mut row, Some(&mut count)) }.map_err(error)?;
        if count == 0 {
            break;
        }
        if result.len() >= MAX_APPS {
            return Err(error("complete app catalog exceeds entry budget"));
        }
        let item = row[0]
            .take()
            .ok_or_else(|| error("Shell returned no enumerated app"))?;
        let identity = text(&item, SIGDN_PARENTRELATIVEPARSING)?;
        let parsing_name = text(&item, SIGDN_DESKTOPABSOLUTEPARSING)?;
        let name = text(&item, SIGDN_NORMALDISPLAY)?;
        if identity.is_empty() || name.is_empty() {
            continue;
        }
        let fingerprint = fingerprint(&item)?;
        let app = CatalogApp {
            app_id: AppId::try_from(format!("windows-aumid:{identity}"))?,
            name,
            kind: AppKind::WindowsShell,
            icon: None,
            can_launch: true,
            unavailable_reason: None,
        };
        result.push(InstalledApp {
            app,
            fingerprint: fingerprint.clone(),
            target: Target {
                parsing_name,
                fingerprint,
            },
        });
    }
    Ok(result)
}
