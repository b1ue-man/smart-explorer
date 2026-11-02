//! How a local media file is handed to its default viewer so that the viewer
//! can step to the neighboring files of the folder (plan:
//! `docs/plaene/2026-10-09-medien-weiterschalten/plan.md`, C1; sources:
//! `docs/refs/medien-weiterschalten-2026-10-09.md`).
//!
//! Classic programs read the folder themselves and get the plain path. The
//! Microsoft Photos app only builds its previous/next list from launch
//! context: its current generation from `ms-photos:viewer?fileName=`, older
//! Photos versions and other packaged apps from a neighboring-files query.
//! The Windows adapter supplies the association facts as plain values.

use crate::types::media_kind_of_name;

/// Package family of the Microsoft Photos app.
pub(in crate::app) const PHOTOS_FAMILY: &str = "Microsoft.Windows.Photos_8wekyb3d8bbwe";

/// First Photos package generation (major version) that ignores a third-party
/// neighboring-files query and is driven through `ms-photos:viewer` instead
/// (Total Commander changelog 25.06.24 and 11.02.25).
const PHOTOS_VIEWER_URI_MAJOR: u32 = 2024;

/// Launch method for one local file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum MediaLaunch {
    /// Unchanged plain `ShellExecute` of the path.
    Shell,
    /// `ShellExecute` of this `ms-photos:viewer` URI.
    PhotosViewer(String),
    /// `Launcher.LaunchFileWithOptionsAsync` with a neighboring-files query
    /// over the parent folder.
    NeighborQuery,
}

/// Default handler of a file extension as the association system reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct DefaultHandler {
    /// `ASSOCSTR_APPID`: AppUserModelID (`<package family>!<app>` when packaged).
    pub(in crate::app) app_id: Option<String>,
    /// `ASSOCSTR_PROGID`: `AppX…` for packaged apps.
    pub(in crate::app) prog_id: Option<String>,
}

impl DefaultHandler {
    fn package_family(&self) -> Option<&str> {
        let app_id = self.app_id.as_deref()?;
        app_id.split_once('!').map(|(family, _)| family)
    }

    fn is_photos(&self) -> bool {
        self.package_family()
            .is_some_and(|family| family.eq_ignore_ascii_case(PHOTOS_FAMILY))
    }

    fn is_packaged(&self) -> bool {
        self.package_family()
            .is_some_and(|family| !family.is_empty())
            || self.prog_id.as_deref().is_some_and(|prog_id| {
                prog_id.len() > 4
                    && prog_id
                        .get(..4)
                        .is_some_and(|p| p.eq_ignore_ascii_case("appx"))
            })
    }
}

/// Chooses the launch for `path` (a Windows path). `photos_major` is asked
/// only when Photos is the default handler and returns the newest installed
/// Photos package generation, `None` when it cannot be determined.
pub(in crate::app) fn plan_media_launch(
    path: &str,
    is_dir: bool,
    handler: &DefaultHandler,
    photos_major: impl FnOnce() -> Option<u32>,
) -> MediaLaunch {
    if is_dir || media_kind_of_name(path).is_none() {
        return MediaLaunch::Shell;
    }
    if handler.is_photos() {
        return match photos_major() {
            Some(major) if major < PHOTOS_VIEWER_URI_MAJOR => MediaLaunch::NeighborQuery,
            _ => MediaLaunch::PhotosViewer(photos_viewer_uri(path)),
        };
    }
    if handler.is_packaged() {
        MediaLaunch::NeighborQuery
    } else {
        MediaLaunch::Shell
    }
}

/// `ms-photos:viewer?fileName=` with the path percent-encoded as UTF-8; only
/// the RFC 3986 unreserved characters stay literal.
pub(in crate::app) fn photos_viewer_uri(path: &str) -> String {
    let mut uri = String::from("ms-photos:viewer?fileName=");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// Major version of a package full name
/// (`<name>_<major>.<minor>.<build>.<revision>_<arch>_<resource>_<publisher>`).
pub(in crate::app) fn package_major_version(full_name: &str) -> Option<u32> {
    let version = full_name.split('_').nth(1)?;
    version.split('.').next()?.parse().ok()
}

/// Newest major version among installed package full names of one family.
pub(in crate::app) fn newest_major_version<'a>(
    full_names: impl IntoIterator<Item = &'a str>,
) -> Option<u32> {
    full_names
        .into_iter()
        .filter_map(package_major_version)
        .max()
}

#[cfg(test)]
#[path = "media_navigation_task_tests.rs"]
mod tests;
