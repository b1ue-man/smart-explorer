//! The KDE Plasma wallpaper (U34-4, C2.6): read-only from
//! `$XDG_CONFIG_HOME/plasma-org.kde.plasma.desktop-appletsrc`, the group
//! `[Containments][<N>][Wallpaper][org.kde.image][General]` of the first
//! desktop containment (`plugin=org.kde.plasma.folder` or
//! `org.kde.desktopcontainment`) whose `lastScreen` is the index of the
//! monitor in the session's list, else of the first containment that has a
//! wallpaper group. `Image` is a `file:///` URL or a path, `FillMode` a number
//! of Qt's `Image` (0 stretch, 1 fit, 2 fill, 3 tile, 6 pad), `Color`
//! `r,g,b`. The placement in this file is assumption A43; a Plasma that keeps
//! it elsewhere gives the colour and a note.
use std::path::PathBuf;

use super::wallpaper::{failed, file_url_to_path, ok, Reading, SettingsSource};
use crate::desktop::os::wallpaper_image::{FitMode, DEFAULT_COLOR};

const FILE_NAME: &str = "plasma-org.kde.plasma.desktop-appletsrc";

/// One `[a][b]…` group of an INI file with its entries.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Group {
    pub(crate) path: Vec<String>,
    pub(crate) entries: Vec<(String, String)>,
}

impl Group {
    fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }
}

/// The groups of an INI text; entries before the first header are dropped.
pub(crate) fn parse_groups(text: &str) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(header) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            groups.push(Group {
                path: header.split("][").map(str::to_string).collect(),
                entries: Vec::new(),
            });
        } else if let (Some(group), Some((key, value))) = (groups.last_mut(), line.split_once('='))
        {
            group
                .entries
                .push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    groups
}

fn containment<'a>(groups: &'a [Group], number: &str) -> Option<&'a Group> {
    groups
        .iter()
        .find(|group| group.path == ["Containments", number])
}

fn wallpaper_group<'a>(groups: &'a [Group], number: &str) -> Option<&'a Group> {
    groups.iter().find(|group| {
        group.path.len() == 5
            && group.path[0] == "Containments"
            && group.path[1] == number
            && group.path[2] == "Wallpaper"
            && group.path[3] == "org.kde.image"
            && group.path[4] == "General"
    })
}

/// The number of the containment whose wallpaper shows on monitor `index`.
pub(crate) fn containment_for(groups: &[Group], index: Option<usize>) -> Option<String> {
    let numbers: Vec<&str> = groups
        .iter()
        .filter(|group| group.path.len() == 2 && group.path[0] == "Containments")
        .map(|group| group.path[1].as_str())
        .collect();
    let desktop = |number: &&str| {
        containment(groups, number).is_some_and(|group| {
            matches!(
                group.get("plugin"),
                Some("org.kde.plasma.folder" | "org.kde.desktopcontainment")
            )
        })
    };
    let on_screen = |number: &&str| {
        let last = containment(groups, number)
            .and_then(|group| group.get("lastScreen"))
            .and_then(|value| value.parse::<usize>().ok());
        index.is_some() && last == index
    };
    numbers
        .iter()
        .find(|number| {
            desktop(number) && on_screen(number) && wallpaper_group(groups, number).is_some()
        })
        .or_else(|| {
            numbers
                .iter()
                .find(|number| wallpaper_group(groups, number).is_some())
        })
        .map(|number| (*number).to_string())
}

fn fill_mode(value: Option<&str>) -> FitMode {
    match value.and_then(|text| text.parse::<u32>().ok()) {
        Some(0) => FitMode::Stretch,
        Some(1) => FitMode::Fit,
        Some(3 | 4 | 5) => FitMode::Tile,
        Some(6) => FitMode::Center,
        _ => FitMode::Fill,
    }
}

fn color(value: Option<&str>) -> [u8; 3] {
    let Some(value) = value else {
        return DEFAULT_COLOR;
    };
    let parts: Vec<u8> = value
        .split(',')
        .filter_map(|part| part.trim().parse().ok())
        .collect();
    match parts.as_slice() {
        [red, green, blue, ..] => [*red, *green, *blue],
        _ => DEFAULT_COLOR,
    }
}

/// The wallpaper of `monitor` from the Plasma configuration.
pub(super) fn read(source: &dyn SettingsSource, monitor: &str) -> Reading {
    let config = source
        .env("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            source
                .env("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        });
    let Some(config) = config else {
        return failed("KDE: kein Konfigurationsordner (HOME fehlt)");
    };
    let path = config.join(FILE_NAME);
    let Some(text) = source.read_file(&path) else {
        return failed(format!("KDE: {} ist nicht lesbar", path.display()));
    };
    let groups = parse_groups(&text);
    let Some(number) = containment_for(&groups, source.monitor_index(monitor)) else {
        return failed("KDE: die Plasma-Konfiguration nennt keinen Hintergrund");
    };
    let Some(group) = wallpaper_group(&groups, &number) else {
        return failed("KDE: die Plasma-Konfiguration nennt keinen Hintergrund");
    };
    let mode = fill_mode(group.get("FillMode"));
    let rgb = color(group.get("Color"));
    ok(group.get("Image").and_then(file_url_to_path), mode, rgb)
}
