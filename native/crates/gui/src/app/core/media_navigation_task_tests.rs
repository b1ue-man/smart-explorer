use super::*;

fn photos() -> DefaultHandler {
    DefaultHandler {
        app_id: Some(format!("{PHOTOS_FAMILY}!App")),
        prog_id: Some("AppX43hnxtbyyps62jhe9sqpdzxn1790zetc".into()),
    }
}

fn classic() -> DefaultHandler {
    DefaultHandler {
        app_id: Some("IrfanView.Image".into()),
        prog_id: Some("IrfanView.jpg".into()),
    }
}

#[test]
fn media_navigation_task_current_photos_get_the_viewer_uri() {
    let path = r"C:\Bilder\Urlaub 2026\Strand.jpg";
    let expected = MediaLaunch::PhotosViewer(photos_viewer_uri(path));
    assert_eq!(
        plan_media_launch(path, false, &photos(), || Some(2025)),
        expected
    );
    assert_eq!(
        plan_media_launch(path, false, &photos(), || Some(2024)),
        expected
    );
    // An unknown package version uses the current method.
    assert_eq!(plan_media_launch(path, false, &photos(), || None), expected);
}

#[test]
fn media_navigation_task_older_photos_and_store_apps_get_a_neighbor_query() {
    let path = r"C:\Bilder\a.png";
    assert_eq!(
        plan_media_launch(path, false, &photos(), || Some(2023)),
        MediaLaunch::NeighborQuery
    );
    let media_player = DefaultHandler {
        app_id: Some("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic".into()),
        prog_id: None,
    };
    assert_eq!(
        plan_media_launch(r"C:\Musik\Lied.mp3", false, &media_player, || None),
        MediaLaunch::NeighborQuery
    );
    let packaged_by_prog_id = DefaultHandler {
        app_id: None,
        prog_id: Some("AppXk0g4vb8gvt7b93tg50ybcy892pge6jmt".into()),
    };
    assert_eq!(
        plan_media_launch(r"C:\Videos\Film.mkv", false, &packaged_by_prog_id, || None),
        MediaLaunch::NeighborQuery
    );
}

#[test]
fn media_navigation_task_classic_programs_folders_and_documents_stay_plain() {
    let asked = std::cell::Cell::new(false);
    let ask = || {
        asked.set(true);
        Some(2025)
    };
    assert_eq!(
        plan_media_launch(r"C:\Bilder\a.jpg", false, &classic(), ask),
        MediaLaunch::Shell
    );
    assert!(
        !asked.get(),
        "the Photos version is only queried for Photos"
    );
    assert_eq!(
        plan_media_launch(r"C:\Bilder\Ordner.jpg", true, &photos(), || Some(2025)),
        MediaLaunch::Shell
    );
    assert_eq!(
        plan_media_launch(r"C:\Texte\Brief.pdf", false, &photos(), || Some(2025)),
        MediaLaunch::Shell
    );
    assert_eq!(
        plan_media_launch(r"C:\Bilder\.jpg", false, &photos(), || Some(2025)),
        MediaLaunch::Shell
    );
    assert_eq!(
        plan_media_launch(
            r"C:\Bilder\a.jpg",
            false,
            &DefaultHandler::default(),
            || None
        ),
        MediaLaunch::Shell
    );
    // A malformed or non-ASCII ProgID is no package marker and never panics.
    let odd = DefaultHandler {
        app_id: Some("!".into()),
        prog_id: Some("pppÄx".into()),
    };
    assert_eq!(
        plan_media_launch(r"C:\a.jpg", false, &odd, || None),
        MediaLaunch::Shell
    );
}

#[test]
fn media_navigation_task_viewer_uri_encodes_every_reserved_byte() {
    assert_eq!(
        photos_viewer_uri(r"C:\Fotos\Ä ö#1%&+,;=?.JPG"),
        "ms-photos:viewer?fileName=C%3A%5CFotos%5C%C3%84%20%C3%B6%231%25%26%2B%2C%3B%3D%3F.JPG"
    );
    assert_eq!(
        photos_viewer_uri(r"\\nas\share\Bild-1_a~b.png"),
        "ms-photos:viewer?fileName=%5C%5Cnas%5Cshare%5CBild-1_a~b.png"
    );
}

#[test]
fn media_navigation_task_package_versions_pick_the_newest_generation() {
    assert_eq!(
        package_major_version("Microsoft.Windows.Photos_2025.11090.12001.0_x64__8wekyb3d8bbwe"),
        Some(2025)
    );
    assert_eq!(package_major_version("Microsoft.Windows.Photos"), None);
    assert_eq!(package_major_version("Name_x.1.2.3_x64__id"), None);
    assert_eq!(
        newest_major_version([
            "Microsoft.Windows.Photos_2023.11110.8002.0_x64__8wekyb3d8bbwe",
            "Microsoft.Windows.Photos_2025.11030.12002.0_x64__8wekyb3d8bbwe",
            "broken",
        ]),
        Some(2025)
    );
    assert_eq!(newest_major_version(std::iter::empty::<&str>()), None);
}

#[test]
fn media_navigation_task_media_kinds_match_the_android_entry_kinds() {
    use crate::types::{media_kind_of_ext, media_kind_of_name, MediaKind};
    for ext in [
        "jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "bmp", "svg", "tif", "tiff", "avif",
        "ico", "dng",
    ] {
        assert_eq!(media_kind_of_ext(ext), Some(MediaKind::Image), "{ext}");
    }
    for ext in [
        "mp4", "mkv", "mov", "avi", "webm", "m4v", "3gp", "wmv", "flv", "mpg", "mpeg", "ts",
    ] {
        assert_eq!(media_kind_of_ext(ext), Some(MediaKind::Video), "{ext}");
    }
    for ext in [
        "mp3", "flac", "wav", "ogg", "opus", "m4a", "aac", "wma", "mid", "midi", "amr",
    ] {
        assert_eq!(media_kind_of_ext(ext), Some(MediaKind::Audio), "{ext}");
    }
    for ext in ["pdf", "txt", "zip", "apk", ""] {
        assert_eq!(media_kind_of_ext(ext), None, "{ext}");
    }
    assert_eq!(
        media_kind_of_name(r"C:\A.B\Foto.JPG"),
        Some(MediaKind::Image)
    );
    assert_eq!(
        media_kind_of_name("/home/u/clip.Mp4"),
        Some(MediaKind::Video)
    );
    assert_eq!(media_kind_of_name("/home/u/.jpg"), None);
    assert_eq!(media_kind_of_name("/home/u.jpg/datei"), None);
    assert_eq!(media_kind_of_name("ohne-endung."), None);
}
