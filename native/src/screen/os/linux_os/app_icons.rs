//! Linux desktop-entry identities and Icon Theme 0.13 lookup, with host-only SVG.
//! `Exec=` is never read. Every path comes from the host's installed catalog/theme.
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use crate::screen::core::app_icon::{self, AuthorizedIconEntry, IconReply, MAX_ICON_BYTES};
use crate::screen::core::window_list::{ListSource, WindowEntry, WindowList};

const OUTPUT_SIDE: u32 = 128;
const MAX_CATALOG_ENTRIES: usize = 8192;

#[derive(Clone)]
struct DesktopIcon {
    id: String,
    startup_class: Option<String>,
    icon: String,
}

fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn read_bounded(path: &Path) -> io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_ICON_BYTES as u64 {
        return Err(app_icon::invalid("icon source exceeds byte budget"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_ICON_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_ICON_BYTES {
        return Err(app_icon::invalid("icon source grew beyond byte budget"));
    }
    Ok(bytes)
}

fn ini(text: &str, section: &str, key: &str) -> Option<String> {
    let mut active = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            active = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) == Some(section);
        } else if active {
            if let Some((name, value)) = line.split_once('=') {
                if name.trim() == key {
                    return Some(value.trim().into());
                }
            }
        }
    }
    None
}

fn catalog(dirs: &[PathBuf]) -> Vec<DesktopIcon> {
    let mut found = BTreeMap::new();
    let mut scanned = 0usize;
    for base in dirs {
        let mut base_ids = HashSet::new();
        let root = base.join("applications");
        let mut pending = vec![root.clone()];
        while let Some(dir) = pending.pop() {
            let children = match fs::read_dir(&dir) {
                Ok(children) => children,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => return Vec::new(),
            };
            for child in children {
                let Ok(child) = child else {
                    return Vec::new();
                };
                scanned += 1;
                // An incomplete catalog cannot prove uniqueness; keep every list entry neutral.
                if scanned > MAX_CATALOG_ENTRIES {
                    return Vec::new();
                }
                let path = child.path();
                let Ok(kind) = child.file_type() else {
                    return Vec::new();
                };
                // Do not follow directory links or recurse outside applications/.
                if kind.is_dir() {
                    if path.components().count() >= root.components().count() + 8 {
                        return Vec::new();
                    }
                    pending.push(path);
                    continue;
                }
                if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
                    continue;
                }
                let Some(id) = path
                    .strip_prefix(&root)
                    .ok()
                    .and_then(|p| p.to_str())
                    .map(|s| s.replace('/', "-"))
                else {
                    continue;
                };
                if !base_ids.insert(id.clone()) {
                    return Vec::new();
                }
                if found.contains_key(&id) {
                    continue;
                }
                // Record tombstones too: Hidden=true masks a system entry with the same ID.
                let Some(text) = read_bounded(&path)
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                else {
                    return Vec::new();
                };
                let entry = (|| {
                    if ini(&text, "Desktop Entry", "Hidden").as_deref() == Some("true")
                        || ini(&text, "Desktop Entry", "Type").as_deref() != Some("Application")
                    {
                        return None;
                    }
                    Some(DesktopIcon {
                        id: id.clone(),
                        startup_class: ini(&text, "Desktop Entry", "StartupWMClass"),
                        icon: ini(&text, "Desktop Entry", "Icon").unwrap_or_default(),
                    })
                })();
                found.insert(id, entry);
            }
        }
    }
    found.into_values().flatten().collect()
}

fn desktop_for<'a>(
    entries: &'a [DesktopIcon],
    id: &str,
    wm_class: bool,
) -> Option<&'a DesktopIcon> {
    let full = if id.ends_with(".desktop") {
        id.to_string()
    } else {
        format!("{id}.desktop")
    };
    let mut candidates = entries.iter().filter(|entry| {
        entry.id == full || (wm_class && entry.startup_class.as_deref() == Some(id))
    });
    let entry = candidates.next()?;
    candidates.next().is_none().then_some(entry)
}

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
    if let Ok(connection) = zbus::blocking::connection::Builder::session().and_then(|builder| {
        builder
            .method_timeout(std::time::Duration::from_millis(500))
            .build()
    }) {
        if let Ok(proxy) = zbus::blocking::Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.Settings",
        ) {
            if let Ok(value) = proxy.call::<_, _, zbus::zvariant::OwnedValue>(
                "Read",
                &("org.gnome.desktop.interface", "icon-theme"),
            ) {
                if let Ok(theme) = <String>::try_from(value) {
                    if safe_relative(&theme) {
                        return theme;
                    }
                }
            }
        }
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

/// EWMH ARGB fallback only after the current window class proves this desktop mapping.
fn window_icon(window: &WindowEntry, desktop: &DesktopIcon) -> Option<IconReply> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    let xid: u32 = window.id.parse().ok()?;
    let (connection, _) = x11rb::rust_connection::RustConnection::connect(None).ok()?;
    let class = connection
        .get_property(false, xid, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)
        .ok()?
        .reply()
        .ok()?;
    if class.bytes_after != 0 {
        return None;
    }
    let class = super::x11_windows::app_of_wm_class(&class.value);
    let stem = desktop.id.strip_suffix(".desktop").unwrap_or(&desktop.id);
    if class != stem && desktop.startup_class.as_deref() != Some(class.as_str()) {
        return None;
    }
    if let Some(expected_pid) = window.pid {
        let atom = connection
            .intern_atom(false, b"_NET_WM_PID")
            .ok()?
            .reply()
            .ok()?
            .atom;
        let property = connection
            .get_property(false, xid, atom, AtomEnum::CARDINAL, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        if property.value32()?.next() != Some(expected_pid) {
            return None;
        }
    }
    let atom = connection
        .intern_atom(false, b"_NET_WM_ICON")
        .ok()?
        .reply()
        .ok()?
        .atom;
    // The property itself is bounded before x11rb allocates its returned pixel words.
    let property = connection
        .get_property(false, xid, atom, AtomEnum::CARDINAL, 0, 65_538)
        .ok()?
        .reply()
        .ok()?;
    if property.bytes_after != 0 {
        return None;
    }
    let words: Vec<u32> = property.value32()?.collect();
    let mut at = 0usize;
    let mut chosen: Option<(u32, u32, &[u32])> = None;
    while let Some(header) = words.get(at..at.checked_add(2)?) {
        let (width, height) = (header[0], header[1]);
        let pixels = app_icon::checked_pixels(width, height).ok()?;
        let end = at.checked_add(2)?.checked_add(pixels)?;
        let content = words.get(at + 2..end)?;
        let distance = width.abs_diff(OUTPUT_SIDE) + height.abs_diff(OUTPUT_SIDE);
        if chosen
            .is_none_or(|(w, h, _)| distance < w.abs_diff(OUTPUT_SIDE) + h.abs_diff(OUTPUT_SIDE))
        {
            chosen = Some((width, height, content));
        }
        at = end;
    }
    if at != words.len() {
        return None;
    }
    let (width, height, content) = chosen?;
    let mut rgba = Vec::with_capacity(content.len() * 4);
    for pixel in content {
        rgba.extend_from_slice(&[
            (pixel >> 16) as u8,
            (pixel >> 8) as u8,
            *pixel as u8,
            (pixel >> 24) as u8,
        ]);
    }
    let png = super::app_icon_raster::rgba_png(width, height, &rgba).ok()?;
    IconReply::new(&desktop.id, &png).ok()
}

/// Refresh on every list: theme changes and changed app bytes produce new hashes.
pub(crate) fn window_identity(native: u64) -> io::Result<Option<String>> {
    let xid = u32::try_from(native)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "X11 window ID out of range"))?;
    let list = super::x11_windows::list()?;
    let id = super::x11_windows::window_id(xid);
    let Some(window) = list.windows.iter().find(|window| window.id == id) else {
        return Ok(None);
    };
    let dirs = super::app_units::data_dirs(&|name| std::env::var(name).ok());
    let entries = catalog(&dirs);
    let source = window.app_id.as_deref().unwrap_or(&window.app);
    Ok(desktop_for(&entries, source, true).map(|entry| entry.id.clone()))
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
