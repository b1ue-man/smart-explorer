//! DSK-M5 checks of the Windows adapter's decisions without the system: the
//! wallpaper per monitor against a fake `IDesktopWallpaper` and the jump
//! list's entries for a tile model.
use std::path::Path;
use std::time::Duration;

use super::jump_list;
use super::monitors::MonitorInfo;
use super::wallpaper::{self, WallpaperSource};
use crate::desktop::core::tiles::{
    self, DeviceTile, RoomGroup, ShareState, TileCode, TileModel, TileOpen,
};
use crate::desktop::os::wallpaper_image::{FitMode, DEFAULT_COLOR};
use crate::share::PeerOpenTarget;

/// `IDesktopWallpaper` as a table.
struct FakeWallpaper {
    ids: Vec<(String, Option<[i32; 4]>)>,
    images: Vec<(String, String)>,
    position: Result<i32, String>,
    color: Result<u32, String>,
    tick: Option<Duration>,
}

impl WallpaperSource for FakeWallpaper {
    fn monitor_ids(&self) -> Vec<(String, Option<[i32; 4]>)> {
        self.ids.clone()
    }

    fn wallpaper(&self, monitor_id: &str) -> Result<String, String> {
        self.images
            .iter()
            .find(|(id, _)| id == monitor_id)
            .map(|(_, path)| path.clone())
            .ok_or_else(|| format!("unbekannt: {monitor_id}"))
    }

    fn position(&self) -> Result<i32, String> {
        self.position.clone()
    }

    fn background_color(&self) -> Result<u32, String> {
        self.color.clone()
    }

    fn slideshow_tick(&self) -> Option<Duration> {
        self.tick
    }
}

fn monitor(path: Option<&str>, rect: [i32; 4]) -> MonitorInfo {
    MonitorInfo {
        key: "k".into(),
        device_path: path.map(str::to_string),
        gdi_name: r"\\.\DISPLAY9".into(),
        description: String::new(),
        primary: false,
        rect,
        work: rect,
    }
}

#[test]
fn desktop_task_windows_wallpaper_per_monitor() {
    const LEFT: &str = r"\\?\DISPLAY#AAA0001#5&1&UID1#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";
    const RIGHT: &str = r"\\?\DISPLAY#BBB0002#5&1&UID2#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}";
    let left_rect = [0, 0, 1920, 1080];
    let right_rect = [1920, 0, 4480, 1440];
    let mut fake = FakeWallpaper {
        ids: vec![
            (LEFT.into(), Some(left_rect)),
            (RIGHT.into(), Some(right_rect)),
            (r"\\?\DISPLAY#OLD#gone".into(), None),
        ],
        images: vec![
            (LEFT.into(), r"C:\Bilder\links.jpg".into()),
            (RIGHT.into(), String::new()),
        ],
        position: Ok(4),
        color: Ok(0x0033_2211),
        tick: None,
    };
    // Per monitor: its own image; system-wide fit mode and color.
    let left =
        wallpaper::background_of(&fake, &monitor(Some(&LEFT.to_lowercase()), left_rect)).unwrap();
    assert_eq!(
        left.wallpaper.path.as_deref(),
        Some(Path::new(r"C:\Bilder\links.jpg"))
    );
    assert_eq!(left.wallpaper.mode, FitMode::Fill);
    assert_eq!(left.wallpaper.color, [0x11, 0x22, 0x33]);
    assert_eq!(left.refresh_every, None);
    // A solid color (or Spotlight): no path, the system color.
    let right = wallpaper::background_of(&fake, &monitor(Some(RIGHT), right_rect)).unwrap();
    assert_eq!(right.wallpaper.path, None);
    assert_eq!(right.wallpaper.color, [0x11, 0x22, 0x33]);
    // Without a known device path the rectangle finds the monitor.
    let by_rect = wallpaper::monitor_id(&fake, &monitor(None, right_rect));
    assert_eq!(by_rect.as_deref(), Some(RIGHT));
    // A monitor the interface does not know keeps the system's color.
    let unknown = wallpaper::background_of(&fake, &monitor(None, [5, 5, 6, 6])).unwrap();
    assert_eq!(unknown.wallpaper.path, None);
    assert_eq!(unknown.wallpaper.color, [0x11, 0x22, 0x33]);
    // Every fit mode of `DESKTOP_WALLPAPER_POSITION`; a slideshow's tick.
    for (position, mode) in [
        (0, FitMode::Center),
        (1, FitMode::Tile),
        (2, FitMode::Stretch),
        (3, FitMode::Fit),
        (4, FitMode::Fill),
        (5, FitMode::Span),
    ] {
        fake.position = Ok(position);
        fake.tick = Some(Duration::from_secs(600));
        let read = wallpaper::background_of(&fake, &monitor(Some(LEFT), left_rect)).unwrap();
        assert_eq!(read.wallpaper.mode, mode, "{position}");
        assert_eq!(read.refresh_every, Some(Duration::from_secs(600)));
    }
    // Failed parts keep the defaults; nothing at all is an error.
    fake.position = Err("GetPosition".into());
    fake.color = Err("GetBackgroundColor".into());
    let defaults = wallpaper::background_of(&fake, &monitor(Some(LEFT), left_rect)).unwrap();
    assert_eq!(defaults.wallpaper.mode, FitMode::Fill);
    assert_eq!(defaults.wallpaper.color, DEFAULT_COLOR);
    assert!(wallpaper::background_of(&fake, &monitor(None, [5, 5, 6, 6])).is_err());
    assert_eq!(wallpaper::rgb(0x00FF_8001), [0x01, 0x80, 0xFF]);
}

fn tile(target: PeerOpenTarget, name: &str, code: TileCode) -> DeviceTile {
    let location = format!("{}/", target.endpoint_prefix());
    let open = match (&target, code.opens_files()) {
        (_, true) => TileOpen::Location(location.clone()),
        (PeerOpenTarget::Direct { contact_id }, false) => {
            TileOpen::Page(format!("share-device:{contact_id}"))
        }
        (PeerOpenTarget::RoomDevice { room_id, .. }, false) => {
            TileOpen::Page(format!("share-room:{room_id}"))
        }
    };
    DeviceTile {
        key: tiles::device_key(&target),
        target,
        name: name.into(),
        code,
        status_text: String::new(),
        last_seen: None,
        lan: false,
        online: false,
        write: None,
        location,
        device_id: None,
        folder_path: None,
        open,
    }
}

/// The jump list's entries follow the tiles: Direct devices, then the room
/// devices with their room, the arguments of a double click (an identity
/// change opens the page), and the settings task.
#[test]
fn jump_list_plan_follows_tiles() {
    let direct = |id: &str| PeerOpenTarget::Direct {
        contact_id: id.into(),
    };
    let model = TileModel {
        share_state: ShareState::Live,
        profiles_error: None,
        devices: vec![
            tile(direct("c1"), "Laptop", TileCode::Connected),
            tile(direct("c2"), "Alt", TileCode::IdentityConflict),
        ],
        rooms: vec![RoomGroup {
            room_id: "r1".into(),
            name: "Büro".into(),
            page: "share-room:r1".into(),
            members: vec![tile(
                PeerOpenTarget::RoomDevice {
                    room_id: "r1".into(),
                    device_id: "d 1".into(),
                },
                "Drucker-PC",
                TileCode::Available,
            )],
        }],
        vaults: None,
        empty_actions: Vec::new(),
    };
    let plan = jump_list::plan_for(&model);
    let titles: Vec<&str> = plan
        .devices
        .iter()
        .map(|entry| entry.title.as_str())
        .collect();
    assert_eq!(titles, ["Laptop", "Alt", "Drucker-PC (Büro)"]);
    assert_eq!(
        plan.devices[0].arguments,
        "--open-location share://direct/c1/"
    );
    assert_eq!(plan.devices[1].arguments, "--open-page share-device:c2");
    assert!(
        plan.devices[2].arguments.starts_with("--open-location "),
        "{}",
        plan.devices[2].arguments
    );
    assert_eq!(plan.task.title, jump_list::SETTINGS_TASK);
    assert_eq!(plan.task.arguments, "--open-page settings-integration");
    assert_eq!(jump_list::quote("a b"), "\"a b\"");
    assert_eq!(jump_list::quote(r#"x"y"#), r#""x\"y""#);
    assert_eq!(jump_list::quote(r"C:\dir\"), r"C:\dir\");
    assert_eq!(jump_list::quote(r"C:\my dir\"), r#""C:\my dir\\""#);
    assert_eq!(jump_list::quote(""), "\"\"");
}
