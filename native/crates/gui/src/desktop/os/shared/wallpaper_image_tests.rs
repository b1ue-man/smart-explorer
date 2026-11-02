//! DSK-M4: the six fit modes of the wallpaper and the color when no image
//! can be shown (Annahme A9).
use std::path::PathBuf;

use super::{
    format_color, load, parse_color, place, spec_for, Area, FitMode, Loaded, Placement,
    SystemWallpaper, WallpaperSpec, DEFAULT_COLOR,
};
use crate::desktop::core::config::{Background, BackgroundKind};

const MONITOR: Area = Area {
    x: 0.0,
    y: 0.0,
    w: 1000.0,
    h: 500.0,
};

fn placed(dest: Area, uv: Area) -> Option<Placement> {
    Some(Placement {
        dest,
        uv,
        repeat: false,
    })
}

#[test]
fn desktop_task_wallpaper_modes() {
    let full = Area::new(0.0, 0.0, 1.0, 1.0);
    // Strecken: the monitor, the whole image.
    assert_eq!(
        place(FitMode::Stretch, (500, 500), MONITOR, 1.0, None),
        placed(MONITOR, full)
    );
    // Einpassen: the whole image inside, the color left and right.
    assert_eq!(
        place(FitMode::Fit, (500, 500), MONITOR, 1.0, None),
        placed(Area::new(250.0, 0.0, 500.0, 500.0), full)
    );
    // Füllen: the monitor covered, the middle half of the image's height.
    assert_eq!(
        place(FitMode::Fill, (500, 500), MONITOR, 1.0, None),
        placed(MONITOR, Area::new(0.0, 0.25, 1.0, 0.5))
    );
    // Zentrieren: the image's own size, in points.
    assert_eq!(
        place(FitMode::Center, (500, 500), MONITOR, 2.0, None),
        placed(Area::new(375.0, 125.0, 250.0, 250.0), full)
    );
    // A centered image larger than the monitor shows its middle.
    assert_eq!(
        place(FitMode::Center, (2000, 1000), MONITOR, 1.0, None),
        placed(MONITOR, Area::new(0.25, 0.25, 0.5, 0.5))
    );
    // Kacheln: one rectangle with repeating texture from the top left.
    assert_eq!(
        place(FitMode::Tile, (500, 500), MONITOR, 1.0, None),
        Some(Placement {
            dest: MONITOR,
            uv: Area::new(0.0, 0.0, 2.0, 1.0),
            repeat: true,
        })
    );
    // Spannen over two monitors side by side: each shows its half.
    let left_span = Area::new(0.0, 0.0, 2000.0, 500.0);
    let right_span = Area::new(-1000.0, 0.0, 2000.0, 500.0);
    assert_eq!(
        place(FitMode::Span, (1000, 500), MONITOR, 1.0, Some(left_span)),
        placed(MONITOR, Area::new(0.0, 0.25, 0.5, 0.5))
    );
    assert_eq!(
        place(FitMode::Span, (1000, 500), MONITOR, 1.0, Some(right_span)),
        placed(MONITOR, Area::new(0.5, 0.25, 0.5, 0.5))
    );
    // Without the other monitors `Span` fills the monitor.
    assert_eq!(
        place(FitMode::Span, (500, 500), MONITOR, 1.0, None),
        place(FitMode::Fill, (500, 500), MONITOR, 1.0, None)
    );
    // An empty image or monitor draws nothing.
    assert_eq!(place(FitMode::Fill, (0, 10), MONITOR, 1.0, None), None);
    let empty = Area::new(0.0, 0.0, 0.0, 10.0);
    assert_eq!(place(FitMode::Fill, (10, 10), empty, 1.0, None), None);
    // The platforms' names of the modes.
    let windows: Vec<FitMode> = (0..6).map(FitMode::from_windows).collect();
    assert_eq!(
        windows,
        [
            FitMode::Center,
            FitMode::Tile,
            FitMode::Stretch,
            FitMode::Fit,
            FitMode::Fill,
            FitMode::Span
        ]
    );
    let gnome: Vec<FitMode> = [
        "zoom",
        "scaled",
        "stretched",
        "wallpaper",
        "centered",
        "spanned",
    ]
    .into_iter()
    .map(FitMode::from_gnome)
    .collect();
    assert_eq!(
        gnome,
        [
            FitMode::Fill,
            FitMode::Fit,
            FitMode::Stretch,
            FitMode::Tile,
            FitMode::Center,
            FitMode::Span
        ]
    );
}

fn background(kind: BackgroundKind, value: Option<&str>) -> Background {
    Background {
        kind,
        value: value.map(str::to_string),
    }
}

#[test]
fn desktop_task_wallpaper_fallback_color() {
    let system = background(BackgroundKind::System, None);
    // Solid color and Windows Spotlight: no path, the system's color.
    let solid = SystemWallpaper {
        path: None,
        mode: FitMode::Fill,
        color: [1, 2, 3],
    };
    assert_eq!(
        spec_for(&system, Some(&solid)),
        WallpaperSpec::Color([1, 2, 3])
    );
    let empty = SystemWallpaper {
        path: Some(PathBuf::new()),
        ..solid.clone()
    };
    assert_eq!(
        spec_for(&system, Some(&empty)),
        WallpaperSpec::Color([1, 2, 3])
    );
    // No system source yet (before DSK-M5/DSK-M6): the default color.
    assert_eq!(spec_for(&system, None), WallpaperSpec::Color(DEFAULT_COLOR));
    let picture = SystemWallpaper {
        path: Some(PathBuf::from("/bilder/berg.jpg")),
        mode: FitMode::Span,
        color: [4, 5, 6],
    };
    assert_eq!(
        spec_for(&system, Some(&picture)),
        WallpaperSpec::Image {
            path: PathBuf::from("/bilder/berg.jpg"),
            mode: FitMode::Span,
            color: [4, 5, 6],
        }
    );
    let color = background(BackgroundKind::Color, Some("#10A0fF"));
    assert_eq!(spec_for(&color, None), WallpaperSpec::Color([16, 160, 255]));
    assert_eq!(parse_color("#10A0fF"), Some([16, 160, 255]));
    assert_eq!(parse_color("10A0FF"), None);
    assert_eq!(parse_color("#10A0F"), None);
    assert_eq!(format_color([16, 160, 255]), "#10A0FF");
    let transparent = background(BackgroundKind::Transparent, None);
    assert_eq!(spec_for(&transparent, None), WallpaperSpec::Transparent);

    // Loading: no path, a missing file and an HDR format give the color.
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(load(None, 4096), Loaded::Color(_)));
    assert!(matches!(
        load(Some(PathBuf::new().as_path()), 4096),
        Loaded::Color(_)
    ));
    let missing = dir.path().join("fehlt.png");
    assert!(matches!(load(Some(&missing), 4096), Loaded::Color(_)));
    let jxr = dir.path().join("hdr.jxr");
    std::fs::write(&jxr, b"II\xBC\x01\x08\x00\x00\x00 kein Bild").unwrap();
    assert!(matches!(load(Some(&jxr), 4096), Loaded::Color(_)));
    // A real image is decoded and kept within the texture limit.
    let png = dir.path().join("klein.png");
    image::RgbaImage::from_pixel(4, 2, image::Rgba([10, 20, 30, 255]))
        .save(&png)
        .unwrap();
    match load(Some(&png), 4096) {
        Loaded::Image(image) => {
            assert_eq!((image.width, image.height), (4, 2));
            assert_eq!(&image.rgba[..4], &[10, 20, 30, 255]);
        }
        other => panic!("PNG nicht dekodiert: {other:?}"),
    }
    match load(Some(&png), 2) {
        Loaded::Image(image) => {
            assert_eq!((image.width, image.height), (2, 1));
            assert_eq!(image.rgba.len(), 8);
        }
        other => panic!("PNG nicht verkleinert: {other:?}"),
    }
}
