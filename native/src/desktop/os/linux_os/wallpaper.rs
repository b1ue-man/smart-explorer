//! The system wallpaper on Linux (U34-4, C2.6): where each desktop keeps it
//! and how that becomes a [`SystemBackground`] for one monitor. The sources
//! sit behind [`SettingsSource`] (`gsettings`, files, `xfconf-query`, the X11
//! root pixmap), so the tables are checked with stand-ins.
//!
//! The order of the check is by `XDG_CURRENT_DESKTOP`: GNOME (also
//! `ubuntu:GNOME` and Unity), `X-Cinnamon`, `MATE`, `KDE`, `XFCE`; any other
//! desktop on X11 reads the root pixmap, on Wayland it is an error. A source
//! that cannot be read, a missing image or a format without decoder is not
//! fatal: the surface paints the colour, and the settings show the reason
//! ("Hintergrundbild kann nicht angezeigt werden: …").
use std::path::{Path, PathBuf};

use super::wallpaper_kde;
use crate::app::desktop_surface_host::SystemBackground;
use crate::desktop::os::wallpaper_image::{parse_color, FitMode, SystemWallpaper, DEFAULT_COLOR};

/// The note for a GNOME slideshow (`DESKTOP-GNOME-XML-WALLPAPER`).
pub(crate) const GNOME_SLIDESHOW: &str = "GNOME-Diashows werden nicht ausgewertet";
pub(crate) const NO_ROOT_PIXMAP: &str = "Kein Systemhintergrund gesetzt";

/// The image of the X11 root window as a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RootImage {
    pub(crate) path: PathBuf,
}

/// Where the settings of the desktops are read from.
pub(crate) trait SettingsSource {
    /// `XDG_CONFIG_HOME`, `XDG_CURRENT_DESKTOP`, `HOME`, `DISPLAY`,
    /// `WAYLAND_DISPLAY`.
    fn env(&self, name: &str) -> Option<String>;
    /// `gsettings get <schema> <key>`, the text as printed.
    fn gsettings(&self, schema: &str, key: &str) -> Option<String>;
    fn read_file(&self, path: &Path) -> Option<String>;
    /// `xfconf-query -c <channel> -p <property>`.
    fn xfconf(&self, channel: &str, property: &str) -> Option<String>;
    /// The root pixmap saved as a file; `Err` names why not.
    fn root_pixmap(&self, monitor: &str) -> Result<RootImage, String>;
    /// The position of the monitor in the session's list (0-based).
    fn monitor_index(&self, monitor: &str) -> Option<usize>;
}

/// What a read of the system wallpaper gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reading {
    pub(crate) background: Result<SystemBackground, String>,
    /// Why the image does not show (the reason of an error, or a source
    /// without an image), for the settings.
    pub(crate) note: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Desktop {
    Gnome,
    Cinnamon,
    Mate,
    Kde,
    Xfce,
    Other,
}

fn desktop_of(source: &dyn SettingsSource) -> Desktop {
    let value = source.env("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let has = |name: &str| {
        value
            .split(':')
            .any(|part| part.trim().eq_ignore_ascii_case(name))
    };
    if has("GNOME") || has("Unity") {
        Desktop::Gnome
    } else if has("X-Cinnamon") {
        Desktop::Cinnamon
    } else if has("MATE") {
        Desktop::Mate
    } else if has("KDE") {
        Desktop::Kde
    } else if has("XFCE") {
        Desktop::Xfce
    } else {
        Desktop::Other
    }
}

/// Whether this session has no desktop with a setting of its own and shows
/// the X11 root pixmap instead (the case that costs an image read).
pub(super) fn uses_root_pixmap(source: &dyn SettingsSource) -> bool {
    desktop_of(source) == Desktop::Other && is_x11(source)
}

fn is_x11(source: &dyn SettingsSource) -> bool {
    source
        .env("WAYLAND_DISPLAY")
        .is_none_or(|value| value.is_empty())
        && source.env("DISPLAY").is_some_and(|value| !value.is_empty())
}

/// A `gsettings` string without its quotes (`'a'` or `"it's"`); anything
/// else stays as it is.
pub(crate) fn unquote(text: &str) -> String {
    let text = text.trim();
    let text = text.strip_prefix("@s").map_or(text, str::trim_start);
    let quoted = text.len() >= 2
        && matches!(text.chars().next(), Some('\'' | '"'))
        && text.chars().next() == text.chars().next_back();
    if !quoted {
        return text.to_string();
    }
    let mut out = String::new();
    let mut chars = text[1..text.len() - 1].chars();
    while let Some(character) = chars.next() {
        if character == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(character);
        }
    }
    out
}

/// The path of a `file:///…` URL with its percent escapes decoded; a plain
/// absolute path stays; every other scheme is no file.
pub(crate) fn file_url_to_path(value: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let Some(rest) = value.strip_prefix("file://") else {
        return value.starts_with('/').then(|| PathBuf::from(value));
    };
    // `file://localhost/…` and `file:///…`: the path starts at the next `/`.
    let path = &rest[rest.find('/')?..];
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|byte| (*byte as char).to_digit(16));
        if bytes[index] == b'%' {
            if let (Some(high), Some(low)) = (hex(index + 1), hex(index + 2)) {
                decoded.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(decoded)))
}

fn color_of(text: &str) -> [u8; 3] {
    parse_color(&unquote(text)).unwrap_or(DEFAULT_COLOR)
}

pub(super) fn ok(path: Option<PathBuf>, mode: FitMode, color: [u8; 3]) -> Reading {
    Reading {
        background: Ok(SystemBackground {
            wallpaper: SystemWallpaper { path, mode, color },
            refresh_every: None,
        }),
        note: None,
    }
}

pub(super) fn failed(reason: impl Into<String>) -> Reading {
    let reason = reason.into();
    Reading {
        background: Err(reason.clone()),
        note: Some(reason),
    }
}

/// GNOME, Cinnamon and MATE: schema, the key of the image and whether it is
/// a URL (MATE keeps a plain path).
fn gsettings_desktop(
    source: &dyn SettingsSource,
    schema: &str,
    picture_key: &str,
    dark_allowed: bool,
) -> Reading {
    let get = |key: &str| source.gsettings(schema, key).map(|text| unquote(&text));
    let dark = dark_allowed
        && source
            .gsettings("org.gnome.desktop.interface", "color-scheme")
            .is_some_and(|text| unquote(&text) == "prefer-dark");
    let mut picture = None;
    if dark {
        picture = get("picture-uri-dark").filter(|value| !value.is_empty());
    }
    let picture = picture.or_else(|| get(picture_key));
    let Some(picture) = picture else {
        return failed(format!(
            "{schema} ist nicht lesbar (gsettings fehlt oder ohne Schema)"
        ));
    };
    let option = get("picture-options").unwrap_or_default();
    let color = source
        .gsettings(schema, "primary-color")
        .map_or(DEFAULT_COLOR, |text| color_of(&text));
    let mode = FitMode::from_gnome(&option);
    if picture.is_empty() || option == "none" {
        return ok(None, mode, color);
    }
    let Some(path) = file_url_to_path(&picture) else {
        return failed(format!("Hintergrundquelle {picture:?} ist keine Datei"));
    };
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
    {
        let mut reading = ok(None, mode, color);
        reading.note = Some(GNOME_SLIDESHOW.into());
        return reading;
    }
    ok(Some(path), mode, color)
}

/// Xfce: `/backdrop/screen0/monitor<key>/workspace0/…`, else `monitor0`.
fn xfce(source: &dyn SettingsSource, monitor: &str) -> Reading {
    let read = |name: &str| {
        [format!("monitor{monitor}"), "monitor0".to_string()]
            .iter()
            .find_map(|node| {
                let property = format!("/backdrop/screen0/{node}/workspace0/{name}");
                source
                    .xfconf("xfce4-desktop", &property)
                    .map(|text| text.trim().to_string())
                    .filter(|text| !text.is_empty())
            })
    };
    let Some(image) = read("last-image") else {
        return failed("xfce4-desktop kennt keinen Hintergrund (xfconf-query fehlt oder leer)");
    };
    let style: u32 = read("image-style")
        .and_then(|text| text.parse().ok())
        .unwrap_or(5);
    let mode = match style {
        1 => FitMode::Center,
        2 => FitMode::Tile,
        3 => FitMode::Stretch,
        4 => FitMode::Fit,
        6 => FitMode::Span,
        _ => FitMode::Fill,
    };
    // `rgba1` prints a header line and then one double per line.
    let channels: Vec<f64> = read("rgba1")
        .map(|text| {
            text.lines()
                .filter_map(|line| line.trim().parse::<f64>().ok())
                .collect()
        })
        .unwrap_or_default();
    let color = match channels.as_slice() {
        [red, green, blue, ..] => {
            [red, green, blue].map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8)
        }
        _ => DEFAULT_COLOR,
    };
    if style == 0 {
        return ok(None, mode, color);
    }
    ok(file_url_to_path(&image), mode, color)
}

/// Reads the system wallpaper of `monitor` from `source`.
pub(crate) fn read(source: &dyn SettingsSource, monitor: &str) -> Reading {
    match desktop_of(source) {
        Desktop::Gnome => {
            gsettings_desktop(source, "org.gnome.desktop.background", "picture-uri", true)
        }
        Desktop::Cinnamon => gsettings_desktop(
            source,
            "org.cinnamon.desktop.background",
            "picture-uri",
            false,
        ),
        Desktop::Mate => {
            gsettings_desktop(source, "org.mate.background", "picture-filename", false)
        }
        Desktop::Kde => wallpaper_kde::read(source, monitor),
        Desktop::Xfce => xfce(source, monitor),
        Desktop::Other => {
            let x11 = source
                .env("WAYLAND_DISPLAY")
                .is_none_or(|value| value.is_empty())
                && source.env("DISPLAY").is_some_and(|value| !value.is_empty());
            if !x11 {
                return failed("Dieser Desktop nennt keinen lesbaren Systemhintergrund");
            }
            match source.root_pixmap(monitor) {
                Ok(image) => ok(Some(image.path), FitMode::Span, DEFAULT_COLOR),
                Err(reason) => failed(reason),
            }
        }
    }
}

#[cfg(test)]
#[path = "wallpaper_tests.rs"]
mod tests;
