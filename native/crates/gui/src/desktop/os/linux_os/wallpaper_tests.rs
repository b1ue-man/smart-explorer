//! Signals `desktop_task_linux_wallpaper_sources` (one stand-in source per
//! desktop of C2.6, assumption A43 for the KDE file) and
//! `desktop_task_linux_wallpaper_file_urls` (`file://` URLs and percent
//! escapes), plus the pure parts of the root pixmap.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::super::wallpaper_root::to_rgb;
use super::*;
use crate::desktop::os::wallpaper_image::{FitMode, DEFAULT_COLOR};

#[derive(Default)]
struct Fake {
    env: HashMap<String, String>,
    gsettings: HashMap<(String, String), String>,
    files: HashMap<PathBuf, String>,
    xfconf: HashMap<String, String>,
    root: Option<PathBuf>,
    monitors: Vec<String>,
}

impl Fake {
    fn on(desktop: &str, display: (&str, &str)) -> Fake {
        let mut fake = Fake::default();
        fake.env
            .insert("XDG_CURRENT_DESKTOP".into(), desktop.into());
        fake.env.insert(display.0.into(), display.1.into());
        fake.env.insert("HOME".into(), "/home/u".into());
        fake
    }

    fn set(&mut self, schema: &str, key: &str, value: &str) {
        self.gsettings
            .insert((schema.into(), key.into()), value.into());
    }
}

impl SettingsSource for Fake {
    fn env(&self, name: &str) -> Option<String> {
        self.env.get(name).cloned()
    }
    fn gsettings(&self, schema: &str, key: &str) -> Option<String> {
        self.gsettings
            .get(&(schema.to_string(), key.to_string()))
            .cloned()
    }
    fn read_file(&self, path: &Path) -> Option<String> {
        self.files.get(path).cloned()
    }
    fn xfconf(&self, channel: &str, property: &str) -> Option<String> {
        assert_eq!(channel, "xfce4-desktop");
        self.xfconf.get(property).cloned()
    }
    fn root_pixmap(&self, _monitor: &str) -> Result<RootImage, String> {
        self.root
            .clone()
            .map(|path| RootImage { path })
            .ok_or_else(|| NO_ROOT_PIXMAP.to_string())
    }
    fn monitor_index(&self, monitor: &str) -> Option<usize> {
        self.monitors.iter().position(|known| known == monitor)
    }
}

fn background(reading: &Reading) -> (Option<PathBuf>, FitMode, [u8; 3]) {
    let found = reading.background.as_ref().expect("a background");
    (
        found.wallpaper.path.clone(),
        found.wallpaper.mode,
        found.wallpaper.color,
    )
}

const GNOME: &str = "org.gnome.desktop.background";

#[test]
fn desktop_task_linux_wallpaper_file_urls() {
    let url = |text: &str| file_url_to_path(text);
    assert_eq!(
        url("file:///usr/share/a.jpg"),
        Some("/usr/share/a.jpg".into())
    );
    assert_eq!(
        url("file:///home/u/Bilder/a%20b%C3%A4.png"),
        Some("/home/u/Bilder/a b\u{e4}.png".into())
    );
    assert_eq!(url("file://localhost/tmp/x.png"), Some("/tmp/x.png".into()));
    assert_eq!(url("/plain/path.png"), Some("/plain/path.png".into()));
    assert_eq!(url("file:///100%25.png"), Some("/100%.png".into()));
    // An incomplete escape stays as it is.
    assert_eq!(url("file:///a%2.png"), Some("/a%2.png".into()));
    assert_eq!(url("http://example.org/a.png"), None);
    assert_eq!(url("relative.png"), None);
    assert_eq!(url("file://nohost"), None);
    assert_eq!(url(""), None);
    // Bytes that are no UTF-8 survive.
    use std::os::unix::ffi::OsStrExt;
    let raw = url("file:///a%FFb").expect("a path");
    assert_eq!(raw.as_os_str().as_bytes(), b"/a\xFFb");
    assert_eq!(unquote("'file:///a.jpg'\n"), "file:///a.jpg");
    assert_eq!(unquote("\"it's\""), "it's");
    assert_eq!(unquote("'a\\'b'"), "a'b");
    assert_eq!(unquote("@s 'x'"), "x");
    assert_eq!(unquote("plain"), "plain");
}

#[test]
fn desktop_task_linux_wallpaper_sources() {
    // GNOME: light and dark image, option, colour.
    let mut gnome = Fake::on("ubuntu:GNOME", ("WAYLAND_DISPLAY", "wayland-0"));
    gnome.set(GNOME, "picture-uri", "'file:///w/day.jpg'");
    gnome.set(GNOME, "picture-uri-dark", "'file:///w/night%20x.jpg'");
    gnome.set(GNOME, "picture-options", "'scaled'");
    gnome.set(GNOME, "primary-color", "'#102030'");
    let day = read(&gnome, "DP-1");
    assert_eq!(
        background(&day),
        (Some("/w/day.jpg".into()), FitMode::Fit, [0x10, 0x20, 0x30])
    );
    assert_eq!(day.note, None);
    gnome.set(
        "org.gnome.desktop.interface",
        "color-scheme",
        "'prefer-dark'",
    );
    assert_eq!(
        background(&read(&gnome, "DP-1")).0,
        Some("/w/night x.jpg".into())
    );
    // A slideshow is no file: colour and the note.
    gnome.set(GNOME, "picture-uri-dark", "'file:///w/show.xml'");
    let show = read(&gnome, "DP-1");
    assert_eq!(background(&show), (None, FitMode::Fit, [0x10, 0x20, 0x30]));
    assert_eq!(show.note.as_deref(), Some(GNOME_SLIDESHOW));
    // No image at all (`none`, empty URI).
    gnome.set(GNOME, "picture-uri-dark", "''");
    gnome.set(GNOME, "picture-uri", "''");
    assert_eq!(background(&read(&gnome, "DP-1")).0, None);
    // gsettings missing: an error with a reason.
    let missing = read(&Fake::on("GNOME", ("DISPLAY", ":0")), "DP-1");
    assert!(missing.background.is_err());
    assert!(missing.note.is_some());

    // Cinnamon and MATE.
    let mut cinnamon = Fake::on("X-Cinnamon", ("DISPLAY", ":0"));
    cinnamon.set(
        "org.cinnamon.desktop.background",
        "picture-uri",
        "'file:///c/a.png'",
    );
    cinnamon.set(
        "org.cinnamon.desktop.background",
        "picture-options",
        "'centered'",
    );
    cinnamon.set(
        "org.cinnamon.desktop.background",
        "primary-color",
        "'#000000'",
    );
    assert_eq!(
        background(&read(&cinnamon, "HDMI-1")),
        (Some("/c/a.png".into()), FitMode::Center, [0, 0, 0])
    );
    let mut mate = Fake::on("MATE", ("DISPLAY", ":0"));
    mate.set("org.mate.background", "picture-filename", "'/m/b.png'");
    mate.set("org.mate.background", "picture-options", "'wallpaper'");
    assert_eq!(
        background(&read(&mate, "HDMI-1")),
        (Some("/m/b.png".into()), FitMode::Tile, DEFAULT_COLOR)
    );

    // KDE: the containment of the monitor's screen, else the first.
    let mut kde = Fake::on("KDE", ("WAYLAND_DISPLAY", "wayland-0"));
    kde.env.insert("XDG_CONFIG_HOME".into(), "/cfg".into());
    kde.monitors = vec!["DP-1".into(), "HDMI-1".into()];
    kde.files.insert(
        "/cfg/plasma-org.kde.plasma.desktop-appletsrc".into(),
        "[Containments][1]\nplugin=org.kde.panel\nlastScreen=0\n\n\
         [Containments][2]\nplugin=org.kde.plasma.folder\nlastScreen=0\n\n\
         [Containments][2][Wallpaper][org.kde.image][General]\n\
         Image=file:///k/one.jpg\nFillMode=2\nColor=1,2,3\n\n\
         [Containments][3]\nplugin=org.kde.desktopcontainment\nlastScreen=1\n\n\
         [Containments][3][Wallpaper][org.kde.image][General]\n\
         Image=/k/two%20.jpg\nFillMode=6\nColor=9, 8, 7\n"
            .into(),
    );
    assert_eq!(
        background(&read(&kde, "DP-1")),
        (Some("/k/one.jpg".into()), FitMode::Fill, [1, 2, 3])
    );
    assert_eq!(
        background(&read(&kde, "HDMI-1")),
        (Some("/k/two%20.jpg".into()), FitMode::Center, [9, 8, 7])
    );
    // A monitor beyond the lists takes the first containment with a wallpaper.
    assert_eq!(
        background(&read(&kde, "unknown")).0,
        Some("/k/one.jpg".into())
    );
    kde.files.clear();
    assert!(read(&kde, "DP-1").background.is_err());
    let groups = wallpaper_kde::parse_groups("[A][B]\nx = 1\n[C]\ny=2\n");
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].path, ["A", "B"]);
    assert_eq!(groups[0].entries, [("x".to_string(), "1".to_string())]);

    // Xfce: the node of the monitor, else monitor0; style and colour.
    let mut xfce = Fake::on("XFCE", ("DISPLAY", ":0"));
    let node = "/backdrop/screen0/monitorHDMI-1/workspace0";
    xfce.xfconf
        .insert(format!("{node}/last-image"), "/x/h.png\n".into());
    xfce.xfconf
        .insert(format!("{node}/image-style"), "4\n".into());
    xfce.xfconf.insert(
        format!("{node}/rgba1"),
        "Value is an array with 4 items:\n\n1.0\n0.0\n0.5\n1.0\n".into(),
    );
    xfce.xfconf.insert(
        "/backdrop/screen0/monitor0/workspace0/last-image".into(),
        "/x/zero.png".into(),
    );
    assert_eq!(
        background(&read(&xfce, "HDMI-1")),
        (Some("/x/h.png".into()), FitMode::Fit, [255, 0, 128])
    );
    let other = background(&read(&xfce, "eDP-1"));
    assert_eq!(
        other.0,
        Some("/x/zero.png".into()),
        "falls back to monitor0"
    );
    assert_eq!(other.1, FitMode::Fill, "style 5 (zoom) when unset");
    assert!(read(&Fake::on("XFCE", ("DISPLAY", ":0")), "eDP-1")
        .background
        .is_err());

    // No desktop of its own: the root pixmap on X11, an error on Wayland.
    let mut bare = Fake::on("none", ("DISPLAY", ":0"));
    assert_eq!(
        read(&bare, "screen").background,
        Err(NO_ROOT_PIXMAP.to_string())
    );
    assert!(uses_root_pixmap(&bare));
    bare.root = Some("/run/user/1/smart-explorer/desktop-root-wallpaper.png".into());
    let (path, mode, _) = background(&read(&bare, "screen"));
    assert!(path.is_some());
    assert_eq!(mode, FitMode::Span);
    let wayland = Fake::on("sway", ("WAYLAND_DISPLAY", "wayland-0"));
    assert!(read(&wayland, "DP-1").background.is_err());
    assert!(!uses_root_pixmap(&wayland));
}

#[test]
fn root_pixmap_pixels_become_rgb() {
    // 2x1, 32 bit, least significant byte first: B G R X per pixel; the row
    // is padded to 12 bytes.
    let data = [
        3, 2, 1, 0, // R=1 G=2 B=3
        6, 5, 4, 0, // R=4 G=5 B=6
        9, 9, 9, 9,
    ];
    assert_eq!(
        to_rgb(&data, (2, 1), 32, 12, true),
        Ok(vec![1, 2, 3, 4, 5, 6])
    );
    // Most significant byte first: X R G B.
    let data = [0, 1, 2, 3, 0, 4, 5, 6];
    assert_eq!(
        to_rgb(&data, (2, 1), 32, 8, false),
        Ok(vec![1, 2, 3, 4, 5, 6])
    );
    // 24 bit, two rows with a stride of 8.
    let data = [3, 2, 1, 6, 5, 4, 0, 0, 9, 8, 7, 12, 11, 10, 0, 0];
    assert_eq!(
        to_rgb(&data, (2, 2), 24, 8, true),
        Ok(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12])
    );
    assert!(to_rgb(&data, (2, 2), 16, 8, true).is_err());
    assert!(to_rgb(&data[..4], (2, 1), 24, 8, true).is_err());
}
