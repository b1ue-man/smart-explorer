//! Themed icons for the se Desktop surface's tiles, as file managers show
//! them (user report 2026-10-27: real symbols, no emoji): icons by
//! freedesktop name, and the icons of local files (folder, `.desktop`
//! starter `Icon=`, MIME type from the shared MIME database).
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use base64::{engine::general_purpose::STANDARD, Engine as _};

use super::{current_theme, ini, read_bounded, resolve, DesktopIcon};
use crate::screen::core::app_icon::{self, IconReply};

/// Shared MIME database files are larger than icons, but bounded.
const MAX_MIME_BYTES: u64 = 8 * 1024 * 1024;

fn data_dirs() -> Vec<PathBuf> {
    super::super::app_units::data_dirs(&|name| std::env::var(name).ok())
}

fn rgba_of(reply: IconReply) -> Option<(u32, Vec<u8>)> {
    let png = STANDARD.decode(reply.png_base64.as_bytes()).ok()?;
    let (width, height, rgba) = app_icon::decode_png(&png).ok()?;
    (width == height).then_some((width, rgba))
}

fn themed(icon: String, dirs: &[PathBuf], theme: &str) -> Option<(u32, Vec<u8>)> {
    let entry = DesktopIcon {
        id: format!("se-desktop:{icon}"),
        startup_class: None,
        icon,
    };
    resolve(&entry, dirs, theme).and_then(rgba_of)
}

/// The first of `names` (freedesktop icon names) the icon theme has.
pub(crate) fn named_icon_rgba(names: &[&str]) -> Option<(u32, Vec<u8>)> {
    let dirs = data_dirs();
    let theme = current_theme();
    names
        .iter()
        .find_map(|name| themed((*name).to_string(), &dirs, &theme))
}

/// A local file's icon: a folder's, a `.desktop` starter's `Icon=`, or the
/// icon of its MIME type with the database's generic fallback.
pub(crate) fn path_icon_rgba(path: &Path) -> Option<(u32, Vec<u8>)> {
    if path.is_dir() {
        return named_icon_rgba(&["folder"]);
    }
    if path
        .extension()
        .is_some_and(|extension| extension == "desktop")
    {
        let icon = read_bounded(path)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|text| ini(&text, "Desktop Entry", "Icon"));
        if let Some(icon) = icon {
            if let Some(found) = themed(icon, &data_dirs(), &current_theme()) {
                return Some(found);
            }
        }
        return named_icon_rgba(&["application-x-executable", "application-x-generic"]);
    }
    let mut names = Vec::new();
    if let Some(mime) = mime_of(path) {
        names.push(mime.replace('/', "-"));
        if let Some(generic) = generic_icon(&mime) {
            names.push(generic);
        }
        if let Some((top, _)) = mime.split_once('/') {
            names.push(format!("{top}-x-generic"));
        }
    }
    names.push("text-x-generic".into());
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    named_icon_rgba(&names)
}

fn mime_lines(file: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for dir in data_dirs() {
        let path = dir.join("mime").join(file);
        let small = std::fs::metadata(&path).is_ok_and(|meta| meta.len() <= MAX_MIME_BYTES);
        if !small {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            lines.extend(text.lines().map(str::to_string));
        }
    }
    lines
}

/// `*.ext` patterns of `globs2` as (weight, lowercase suffix, MIME type).
fn suffix_globs() -> &'static [(u32, String, String)] {
    static GLOBS: OnceLock<Vec<(u32, String, String)>> = OnceLock::new();
    GLOBS.get_or_init(|| {
        mime_lines("globs2")
            .iter()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| {
                let mut parts = line.split(':');
                let weight = parts.next()?.parse().ok()?;
                let mime = parts.next()?.to_string();
                let suffix = parts.next()?.strip_prefix('*')?;
                (!suffix.contains(['*', '?', '['])).then(|| (weight, suffix.to_lowercase(), mime))
            })
            .collect()
    })
}

/// The MIME type of `path` by its name: highest weight, then longest suffix.
fn mime_of(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?.to_lowercase();
    suffix_globs()
        .iter()
        .filter(|(_, suffix, _)| name.ends_with(suffix.as_str()))
        .max_by_key(|(weight, suffix, _)| (*weight, suffix.len()))
        .map(|(_, _, mime)| mime.clone())
}

/// The database's generic icon name for `mime` (`generic-icons`).
fn generic_icon(mime: &str) -> Option<String> {
    static GENERIC: OnceLock<Vec<(String, String)>> = OnceLock::new();
    GENERIC
        .get_or_init(|| {
            mime_lines("generic-icons")
                .iter()
                .filter_map(|line| {
                    let (mime, icon) = line.split_once(':')?;
                    Some((mime.to_string(), icon.trim().to_string()))
                })
                .collect()
        })
        .iter()
        .find(|(known, _)| known == mime)
        .map(|(_, icon)| icon.clone())
}
