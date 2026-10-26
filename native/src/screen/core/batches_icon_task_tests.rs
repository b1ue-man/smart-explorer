use super::super::window_list::ListSource;
use super::*;

fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[23, 45, 67, 255])
            .unwrap();
    }
    bytes
}

#[test]
fn batches_task_b6_icon_authorizes_only_unique_current_identity() {
    let reply = IconReply::new("app.example", &png()).unwrap();
    let window = WindowEntry {
        id: "window-1".into(),
        title: "Document".into(),
        app: "Example".into(),
        app_id: Some(reply.icon.app_id.clone()),
        icon: Some(reply.icon.clone()),
        pid: Some(42),
        monitor: None,
        minimized: false,
        focused: false,
    };
    let app = AppEntry {
        app_id: reply.icon.app_id.clone(),
        name: "Example".into(),
        icon: Some(reply.icon.clone()),
        pid: Some(42),
    };
    let mut list = WindowList {
        windows: vec![window.clone()],
        apps: vec![app.clone()],
        list_source: ListSource::EnumWindows,
        limits: vec![],
    };
    let window_request = IconRequest {
        target: IconTarget::Window {
            id: window.id.clone(),
        },
        icon: reply.icon.clone(),
    };
    let app_request = IconRequest {
        target: IconTarget::App {
            app_id: app.app_id.clone(),
        },
        icon: reply.icon.clone(),
    };
    assert!(matches!(
        window_request.authorize(&list),
        Ok(AuthorizedIconEntry::Window(_))
    ));
    assert!(matches!(
        app_request.authorize(&list),
        Ok(AuthorizedIconEntry::App(_))
    ));
    list.windows.push(window);
    list.apps.push(app);
    assert!(window_request.authorize(&list).is_err());
    assert!(app_request.authorize(&list).is_err());
    list.windows.truncate(1);
    list.apps.truncate(1);
    list.windows[0].app_id = Some("replacement.app".into());
    list.apps[0].icon.as_mut().unwrap().sha256 = "0".repeat(64);
    assert!(window_request.authorize(&list).is_err());
    assert!(app_request.authorize(&list).is_err());
    list.windows.clear();
    list.apps.clear();
    assert!(window_request.authorize(&list).is_err());
    assert!(app_request.authorize(&list).is_err());
}

#[test]
fn batches_task_b6_icon_reply_binds_app_hash_dimensions_and_decoded_pixels() {
    let bytes = png();
    let reply = IconReply::new("app.example", &bytes).unwrap();
    assert_eq!(reply.bytes_for(&reply.icon).unwrap(), bytes);
    assert_eq!(decode_png(&bytes).unwrap(), (1, 1, vec![23, 45, 67, 255]));
    let mut expected = reply.icon.clone();
    expected.app_id = "different.app".into();
    assert!(reply.bytes_for(&expected).is_err());
    expected = reply.icon.clone();
    expected.width = 2;
    assert!(reply.bytes_for(&expected).is_err());
    let mut altered = reply.clone();
    let mut tampered = bytes.clone();
    *tampered.last_mut().unwrap() ^= 1;
    altered.png_base64 = STANDARD.encode(tampered);
    assert!(altered.bytes_for(&reply.icon).is_err());
    altered.png_base64 = "!invalid".into();
    assert!(altered.bytes_for(&reply.icon).is_err());
    altered.png_base64 = "A".repeat(MAX_ICON_BASE64_BYTES + 1);
    assert!(altered.bytes_for(&reply.icon).is_err());
}

#[test]
fn batches_task_b6_icon_rejects_oversized_ihdr_before_decoding() {
    let mut bytes = png();
    for dimension in [0, MAX_ICON_SIDE + 1, u32::MAX] {
        bytes[16..20].copy_from_slice(&dimension.to_be_bytes());
        assert!(png_dimensions(&bytes).is_err());
        assert!(decode_png(&bytes).is_err());
        assert!(IconReply::new("app.example", &bytes).is_err());
    }
    assert!(png_dimensions(&vec![0; MAX_ICON_BYTES + 1]).is_err());
    assert!(png_dimensions(&bytes[..32]).is_err());
}
