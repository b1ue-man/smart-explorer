//! Bounded installed desktop-entry catalog and unambiguous ID/class lookup.
use super::{app_icon, MAX_ICON_BYTES};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

const MAX_CATALOG_ENTRIES: usize = 8192;

#[derive(Clone)]
pub(super) struct DesktopIcon {
    pub(super) id: String,
    pub(super) startup_class: Option<String>,
    pub(super) icon: String,
}

pub(super) fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

pub(super) fn read_bounded(path: &Path) -> io::Result<Vec<u8>> {
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

pub(super) fn ini(text: &str, section: &str, key: &str) -> Option<String> {
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

pub(super) fn catalog(dirs: &[PathBuf]) -> Vec<DesktopIcon> {
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

pub(super) fn desktop_for<'a>(
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
