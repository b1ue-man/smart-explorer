//! Linux desktop-entry identities and Icon Theme 0.13 lookup, with host-only SVG.
//! `Exec=` is never read. Every path comes from the host's installed catalog/theme.
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::screen::core::app_icon::{self, AuthorizedIconEntry, IconReply, MAX_ICON_BYTES};
use crate::screen::core::window_list::{ListSource, WindowEntry, WindowList};

#[path = "app_icon_catalog.rs"]
mod installed;
#[path = "app_icons_named.rs"]
mod named;
#[path = "app_icon_netwm.rs"]
mod netwm;
use installed::{catalog, desktop_for, ini, read_bounded, safe_relative, DesktopIcon};
pub(crate) use named::{named_icon_rgba, path_icon_rgba};
use netwm::window_icon;
pub(crate) use netwm::window_identity;

const OUTPUT_SIDE: u32 = 128;

fn icon_bases(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        bases.push(PathBuf::from(home).join(".icons"));
    }
    bases.extend(dirs.iter().map(|dir| dir.join("icons")));
    bases
}

/// Host configuration only; no requester-controlled command or argument is executed.
fn current_theme() -> String {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    if std::env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|d| d.split(':').any(|s| s.eq_ignore_ascii_case("KDE")))
    {
        if let Some(theme) = config
            .as_ref()
            .and_then(|c| read_bounded(&c.join("kdeglobals")).ok())
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|s| ini(&s, "Icons", "Theme"))
        {
            if safe_relative(&theme) {
                return theme;
            }
        }
    }
    // GTK's current GSettings value, read directly over the session portal (no subprocess).
    if let Some(theme) = super::app_units::off_runtime(portal_theme) {
        return theme;
    }
    if let Some(config) = config {
        for file in ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"] {
            if let Some(theme) = read_bounded(&config.join(file))
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
                .and_then(|s| ini(&s, "Settings", "gtk-icon-theme-name"))
            {
                if safe_relative(&theme) {
                    return theme;
                }
            }
        }
    }
    "hicolor".into()
}

/// GTK's icon theme from the portal's GSettings mirror (blocking zbus).
fn portal_theme() -> Option<String> {
    let connection = zbus::blocking::connection::Builder::session()
        .and_then(|builder| {
            builder
                .method_timeout(std::time::Duration::from_millis(500))
                .build()
        })
        .ok()?;
    let proxy = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
    )
    .ok()?;
    let value = proxy
        .call::<_, _, zbus::zvariant::OwnedValue>(
            "Read",
            &("org.gnome.desktop.interface", "icon-theme"),
        )
        .ok()?;
    <String>::try_from(value)
        .ok()
        .filter(|theme| safe_relative(theme))
}

fn theme_icon(
    name: &str,
    theme: &str,
    bases: &[PathBuf],
    visited: &mut HashSet<String>,
) -> Option<PathBuf> {
    if !safe_relative(theme) || visited.len() >= 32 || !visited.insert(theme.into()) {
        return None;
    }
    let index = bases.iter().find_map(|base| {
        read_bounded(&base.join(theme).join("index.theme"))
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
    })?;
    let dirs = [
        ini(&index, "Icon Theme", "Directories"),
        ini(&index, "Icon Theme", "ScaledDirectories"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(",");
    let mut nearest: Option<(u32, PathBuf)> = None;
    for dir in dirs.split(',').filter(|dir| safe_relative(dir)) {
        let number = |key: &str, default| {
            ini(&index, dir, key)
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(default)
        };
        let size = number("Size", 0);
        let scale = number("Scale", 1).clamp(1, 16);
        let (low, high) = match ini(&index, dir, "Type").as_deref() {
            Some("Fixed") => (size, size),
            Some("Scalable") => (number("MinSize", size), number("MaxSize", size)),
            _ => (
                size.saturating_sub(number("Threshold", 2)),
                size.saturating_add(number("Threshold", 2)),
            ),
        };
        let low = low.saturating_mul(scale);
        let high = high.saturating_mul(scale).max(low);
        let distance = if OUTPUT_SIDE < low {
            low - OUTPUT_SIDE
        } else {
            OUTPUT_SIDE.saturating_sub(high)
        };
        for base in bases {
            for extension in ["png", "svg"] {
                let path = base
                    .join(theme)
                    .join(dir)
                    .join(format!("{name}.{extension}"));
                if path.is_file() && nearest.as_ref().is_none_or(|(best, _)| distance < *best) {
                    nearest = Some((distance, path));
                }
            }
        }
    }
    if let Some((_, path)) = nearest {
        return Some(path);
    }
    for parent in ini(&index, "Icon Theme", "Inherits")
        .unwrap_or_default()
        .split(',')
    {
        if let Some(path) = theme_icon(name, parent, bases, visited) {
            return Some(path);
        }
    }
    None
}

fn source_path(entry: &DesktopIcon, dirs: &[PathBuf], theme: &str) -> Option<PathBuf> {
    let path = Path::new(&entry.icon);
    if path.is_absolute() {
        return path.is_file().then(|| path.into());
    }
    if !safe_relative(&entry.icon) || path.components().count() != 1 {
        return None;
    }
    let name = entry
        .icon
        .strip_suffix(".png")
        .or_else(|| entry.icon.strip_suffix(".svg"))
        .unwrap_or(&entry.icon);
    let bases = icon_bases(dirs);
    let mut visited = HashSet::new();
    theme_icon(name, theme, &bases, &mut visited)
        .or_else(|| theme_icon(name, "hicolor", &bases, &mut visited))
        .or_else(|| {
            bases.iter().find_map(|base| {
                ["png", "svg"]
                    .iter()
                    .map(|ext| base.join(format!("{name}.{ext}")))
                    .find(|path| path.is_file())
            })
        })
        .or_else(|| {
            dirs.iter().find_map(|base| {
                ["png", "svg"]
                    .iter()
                    .map(|ext| base.join("pixmaps").join(format!("{name}.{ext}")))
                    .find(|path| path.is_file())
            })
        })
}

fn render_svg(app_id: &str, bytes: &[u8]) -> io::Result<IconReply> {
    if bytes.len() > MAX_ICON_BYTES {
        return Err(app_icon::invalid("SVG icon exceeds byte budget"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| app_icon::invalid("SVG is not UTF-8"))?;
    let xml =
        roxmltree::Document::parse(text).map_err(|_| app_icon::invalid("invalid SVG icon"))?;
    // Bound parser/render work, and reject external references before usvg can resolve them.
    if xml.descendants().take(16_385).count() > 16_384 {
        return Err(app_icon::invalid("SVG icon node budget exceeded"));
    }
    for node in xml.descendants().filter(|n| n.is_element()) {
        if node.tag_name().name() == "text" {
            return Err(app_icon::invalid("SVG icon requires disabled font loading"));
        }
        for attribute in node.attributes() {
            if attribute.name() == "href"
                && !attribute.value().starts_with('#')
                && !attribute.value().starts_with("data:image/png;base64,")
            {
                return Err(app_icon::invalid("external SVG reference refused"));
            }
        }
    }
    let mut options = resvg::usvg::Options::default();
    options.image_href_resolver = resvg::usvg::ImageHrefResolver {
        resolve_string: Box::new(|_, _| None),
        resolve_data: Box::new(|mime, data, _| {
            (mime == "image/png" && app_icon::decode_png(&data).is_ok())
                .then_some(resvg::usvg::ImageKind::PNG(data))
        }),
    };
    let tree = resvg::usvg::Tree::from_data(bytes, &options)
        .map_err(|_| app_icon::invalid("SVG icon parse failed"))?;
    let size = tree.size();
    if size.width() < 1.0
        || size.height() < 1.0
        || size.width() > 2048.0
        || size.height() > 2048.0
        || size.width() * size.height() > 4_194_304.0
    {
        return Err(app_icon::invalid("SVG source dimensions exceed budget"));
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(OUTPUT_SIDE, OUTPUT_SIDE)
        .ok_or_else(|| app_icon::invalid("SVG icon allocation failed"))?;
    let scale = (OUTPUT_SIDE as f32 / size.width()).min(OUTPUT_SIDE as f32 / size.height());
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(
        (OUTPUT_SIDE as f32 - size.width() * scale) / 2.0,
        (OUTPUT_SIDE as f32 - size.height() * scale) / 2.0,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    // tiny-skia is premultiplied; PNG carries straight alpha.
    let mut rgba = pixmap.take();
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha != 0 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    let png = super::app_icon_raster::rgba_png(OUTPUT_SIDE, OUTPUT_SIDE, &rgba)?;
    IconReply::new(app_id, &png)
}

fn resolve(entry: &DesktopIcon, dirs: &[PathBuf], theme: &str) -> Option<IconReply> {
    let path = source_path(entry, dirs, theme)?;
    let bytes = read_bounded(&path).ok()?;
    if path.extension().and_then(|s| s.to_str()) == Some("svg") {
        render_svg(&entry.id, &bytes).ok()
    } else {
        super::app_icon_raster::raster_png(&entry.id, &bytes).ok()
    }
}

pub(crate) fn attach(list: &mut WindowList) {
    let dirs = super::app_units::data_dirs(&|name| std::env::var(name).ok());
    let entries = catalog(&dirs);
    let theme = current_theme();
    let wm_class = matches!(
        list.list_source,
        ListSource::Ewmh | ListSource::X11QueryTree
    );
    let mut resolved = BTreeMap::new();
    for window in &mut list.windows {
        let source_id = window.app_id.as_deref().unwrap_or(&window.app);
        let Some(entry) = desktop_for(&entries, source_id, wm_class) else {
            window.icon = None;
            continue;
        };
        window.app_id = Some(entry.id.clone());
        window.icon = resolved
            .entry(entry.id.clone())
            .or_insert_with(|| resolve(entry, &dirs, &theme))
            .as_ref()
            .map(|reply| reply.icon.clone());
        if wm_class && window.icon.is_none() {
            window.icon = window_icon(window, entry).map(|reply| reply.icon);
        }
    }
    for app in &mut list.apps {
        let Some(entry) = desktop_for(&entries, &app.app_id, false) else {
            app.icon = None;
            continue;
        };
        // AppEntry keeps its established app_id; reference uses that exact identity too.
        app.icon = resolve(entry, &dirs, &theme).map(|mut reply| {
            reply.icon.app_id = app.app_id.clone();
            reply.icon
        });
    }
}

pub(crate) fn load(entry: AuthorizedIconEntry<'_>) -> io::Result<Option<IconReply>> {
    let (id, expected, window) = match entry {
        AuthorizedIconEntry::Window(window) => {
            (window.app_id.as_deref(), window.icon.as_ref(), Some(window))
        }
        AuthorizedIconEntry::App(app) => (Some(app.app_id.as_str()), app.icon.as_ref(), None),
    };
    let (Some(id), Some(expected)) = (id, expected) else {
        return Ok(None);
    };
    let dirs = super::app_units::data_dirs(&|name| std::env::var(name).ok());
    let entries = catalog(&dirs);
    let Some(desktop) = desktop_for(&entries, id, false) else {
        return Ok(None);
    };
    let reply = resolve(desktop, &dirs, &current_theme())
        .or_else(|| window.and_then(|window| window_icon(window, desktop)))
        .map(|mut reply| {
            reply.icon.app_id = id.into();
            reply
        });
    Ok(reply.filter(|reply| &reply.icon == expected))
}

/// Exact installed desktop-file ID, reusable by the authorized AppHost catalog.
pub(crate) fn catalog_icon(app_id: &str) -> Option<IconReply> {
    let dirs = super::app_units::data_dirs(&|name| std::env::var(name).ok());
    let entries = catalog(&dirs);
    let entry = desktop_for(&entries, app_id, false)?;
    resolve(entry, &dirs, &current_theme()).map(|mut reply| {
        reply.icon.app_id = app_id.into();
        reply
    })
}

pub(crate) fn attach_apps(apps: &mut [crate::screen::core::window_list::AppEntry]) {
    let dirs = super::app_units::data_dirs(&|name| std::env::var(name).ok());
    let entries = catalog(&dirs);
    let theme = current_theme();
    for app in apps {
        app.icon = desktop_for(&entries, &app.app_id, false)
            .and_then(|entry| resolve(entry, &dirs, &theme))
            .map(|mut reply| {
                reply.icon.app_id = app.app_id.clone();
                reply.icon
            });
    }
}

