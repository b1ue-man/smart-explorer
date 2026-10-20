//! Signal `desktop_task_wallpaper_hint_text` (U34-4): the line under the
//! choice of the background, and the cases that bring it.
use super::*;
use crate::desktop::core::config::{Background, SystemItems};

fn config_with(background: Background) -> DesktopConfig {
    let mut config = DesktopConfig::initial(SystemItems::initial(None, None));
    config.background = background;
    config
}

fn image(path: &str) -> Background {
    Background {
        kind: BackgroundKind::Image,
        value: Some(path.into()),
    }
}

#[test]
fn desktop_task_wallpaper_hint_text() {
    assert_eq!(
        wallpaper_hint_text("Kein Systemhintergrund gesetzt"),
        "Hintergrundbild kann nicht angezeigt werden: Kein Systemhintergrund gesetzt"
    );

    // A configured image: missing, a folder, no image, an image.
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nicht-da.png");
    let reason = wallpaper_reason(&config_with(image(&missing.to_string_lossy())))
        .expect("a missing file is a reason");
    assert!(
        reason.contains("nicht-da.png") && reason.contains("gibt es nicht"),
        "{reason}"
    );
    let reason = wallpaper_reason(&config_with(image(&dir.path().to_string_lossy())))
        .expect("a folder is a reason");
    assert!(reason.contains("ein Ordner"), "{reason}");
    let text = dir.path().join("text.png");
    std::fs::write(&text, b"this is not a png").unwrap();
    let reason = wallpaper_reason(&config_with(image(&text.to_string_lossy())))
        .expect("a file without image is a reason");
    assert!(reason.starts_with("kein lesbares Bild"), "{reason}");
    let good = dir.path().join("good.png");
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 2, 2);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[7u8; 12]).unwrap();
        writer.finish().unwrap();
    }
    std::fs::write(&good, &png).unwrap();
    assert_eq!(
        wallpaper_reason(&config_with(image(&good.to_string_lossy()))),
        None
    );

    // A colour, "transparent" and an image without a path give no hint.
    let colour = Background {
        kind: BackgroundKind::Color,
        value: Some("#102030".into()),
    };
    assert_eq!(wallpaper_reason(&config_with(colour)), None);
    assert_eq!(wallpaper_reason(&config_with(image(""))), None);
}
