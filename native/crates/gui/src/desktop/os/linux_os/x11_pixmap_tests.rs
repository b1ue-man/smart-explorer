//! Signal `desktop_task_x11_root_pixmap_background` (U34-4, assumption A13):
//! a program sets `_XROOTPMAP_ID` (or `ESETROOT_PMAP_ID`) as `feh` does; the
//! pixels become a private PNG file that is read like any wallpaper. Needs
//! Xvfb (suite stage `linux-desktop: x11`, `SE_DESKTOP_TASK_X11=1`).
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, CreateGCAux, PropMode, Rectangle};
use x11rb::wrapper::ConnectionExt as _;

use super::super::wallpaper::{read, RootImage, SettingsSource, NO_ROOT_PIXMAP};
use super::super::wallpaper_system::SystemSource;
use super::super::x11_facts;
use super::super::x11_task_env::{require_task, XServer};
use super::pixmap_id;
use crate::desktop::os::wallpaper_image::FitMode;

/// The settings of a desktop that names none, with the real root pixmap.
struct Bare;

impl SettingsSource for Bare {
    fn env(&self, name: &str) -> Option<String> {
        match name {
            "XDG_CURRENT_DESKTOP" => Some("none".into()),
            other => std::env::var(other).ok(),
        }
    }
    fn gsettings(&self, _schema: &str, _key: &str) -> Option<String> {
        None
    }
    fn read_file(&self, _path: &std::path::Path) -> Option<String> {
        None
    }
    fn xfconf(&self, _channel: &str, _property: &str) -> Option<String> {
        None
    }
    fn root_pixmap(&self, monitor: &str) -> Result<RootImage, String> {
        SystemSource.root_pixmap(monitor)
    }
    fn monitor_index(&self, _monitor: &str) -> Option<usize> {
        None
    }
}

#[test]
#[ignore = "braucht Xvfb (Stufe linux-desktop: x11)"]
fn desktop_task_x11_root_pixmap_background() {
    require_task();
    let x = XServer::start("320x200x24");
    let (conn, root) = x.connect();
    // No program set a pixmap: the error that says so.
    assert_eq!(
        read(&Bare, "screen").background,
        Err(NO_ROOT_PIXMAP.to_string())
    );
    assert_eq!(pixmap_id(), None);

    // A red pixmap, as `feh` sets it.
    let depth = conn.setup().roots[0].root_depth;
    let pixmap = conn.generate_id().unwrap();
    conn.create_pixmap(depth, pixmap, root, 64, 48).unwrap();
    let gc = conn.generate_id().unwrap();
    conn.create_gc(gc, pixmap, &CreateGCAux::new().foreground(0x00FF_0000))
        .unwrap();
    conn.poly_fill_rectangle(
        pixmap,
        gc,
        &[Rectangle {
            x: 0,
            y: 0,
            width: 64,
            height: 48,
        }],
    )
    .unwrap();
    for name in ["ESETROOT_PMAP_ID", "_XROOTPMAP_ID"] {
        let atom = x11_facts::intern(&conn, name).unwrap();
        conn.change_property32(PropMode::REPLACE, root, atom, AtomEnum::PIXMAP, &[pixmap])
            .unwrap();
        conn.flush().unwrap();
        assert_eq!(pixmap_id(), Some(pixmap), "{name}");
        let found = read(&Bare, "screen")
            .background
            .expect("the root pixmap is read");
        assert_eq!(found.wallpaper.mode, FitMode::Span);
        let path = found.wallpaper.path.expect("saved as a file");
        assert!(
            path.ends_with("smart-explorer/desktop-root-wallpaper.png"),
            "{path:?}"
        );
        let image = image::open(&path).expect("a PNG").to_rgb8();
        assert_eq!(image.dimensions(), (64, 48));
        assert_eq!(image.get_pixel(10, 10).0, [255, 0, 0], "{name}");
        // Private: 0600.
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "{mode:o}");
        conn.delete_property(root, atom).unwrap();
        conn.flush().unwrap();
    }
    assert_eq!(
        read(&Bare, "screen").background,
        Err(NO_ROOT_PIXMAP.to_string()),
        "both properties gone"
    );
}
