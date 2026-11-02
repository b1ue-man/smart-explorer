//! Signal of the thumbnail cleanup of vault drives (VLT-M12, K45, S2-V7;
//! stage `vault_unit`, job linux-core: unit): prepared thumbnails with
//! `Thumb::URI` inside and outside the drive folder, in every text chunk
//! kind and cache folder; only those inside go, links are never followed.
use std::fs;
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::Path;

use super::{file_uri_path, remove_for_place};

/// One PNG chunk with its CRC, as libpng writes it.
fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut crc = flate2::Crc::new();
    crc.update(kind);
    crc.update(data);
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc.sum().to_be_bytes());
    out
}

/// A 1×1 PNG with one text chunk before its image data.
pub(super) fn png_with_text(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend(chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]));
    png.extend(chunk(b"tEXt", b"Thumb::MTime\x001700000000"));
    png.extend(chunk(kind, data));
    png.extend(chunk(
        b"IDAT",
        &[0x78, 0x9C, 0x63, 0x60, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01],
    ));
    png.extend(chunk(b"IEND", &[]));
    png
}

/// A thumbnail as GNOME writes it: `Thumb::URI` in a `tEXt` chunk.
pub(super) fn thumbnail(uri: &str) -> Vec<u8> {
    png_with_text(
        b"tEXt",
        &[b"Thumb::URI\0".as_slice(), uri.as_bytes()].concat(),
    )
}

/// `file://` URI of `path`, escaped like GLib: unreserved bytes and `/`
/// stay, everything else is `%XX`.
pub(super) fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(*byte as char)
            }
            other => uri.push_str(&format!("%{other:02X}")),
        }
    }
    uri
}

fn ztxt(uri: &str) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(uri.as_bytes()).unwrap();
    let mut data = b"Thumb::URI\0\0".to_vec();
    data.extend(encoder.finish().unwrap());
    png_with_text(b"zTXt", &data)
}

fn itxt(uri: &str) -> Vec<u8> {
    // Keyword, no compression, empty language and translated keyword.
    let mut data = b"Thumb::URI\0\0\0\0\0".to_vec();
    data.extend_from_slice(uri.as_bytes());
    png_with_text(b"iTXt", &data)
}

fn put(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[test]
fn vault_task_linux_thumbnails_removed_by_uri_prefix() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("thumbnails");
    let vaults = base
        .path()
        .join("run")
        .join("smart-explorer")
        .join("Tresore");
    let place = vaults.join("Mein Tresor");
    let inside = place.join("Bilder").join("Urlaub ä.jpg");
    let sibling = vaults.join("Mein Tresor 2").join("x.jpg");
    let outside = base.path().join("Bilder").join("y.jpg");

    let removed_names = [
        (
            "normal",
            "11111111111111111111111111111111.png",
            thumbnail(&file_uri(&inside)),
        ),
        (
            "large",
            "22222222222222222222222222222222.png",
            ztxt(&file_uri(&inside)),
        ),
        (
            "x-large",
            "33333333333333333333333333333333.png",
            itxt(&file_uri(&place)),
        ),
        (
            "fail/gnome-thumbnail-factory",
            "44444444444444444444444444444444.png",
            thumbnail(&file_uri(&inside).replacen("file://", "file://localhost", 1)),
        ),
    ];
    let kept_names = [
        (
            "xx-large",
            "55555555555555555555555555555555.png",
            thumbnail(&file_uri(&sibling)),
        ),
        (
            "normal",
            "66666666666666666666666666666666.png",
            thumbnail(&file_uri(&outside)),
        ),
        (
            "normal",
            "77777777777777777777777777777777.png",
            b"kein PNG".to_vec(),
        ),
        ("normal", "notes.txt", thumbnail(&file_uri(&inside))),
        (
            "normal",
            "88888888888888888888888888888888.png",
            png_with_text(b"tEXt", b"Thumb::Size\x0012"),
        ),
    ];
    for (folder, name, bytes) in removed_names.iter().chain(kept_names.iter()) {
        put(&root.join(folder).join(name), bytes);
    }
    // A link in the cache and a linked folder: never followed.
    let target = base
        .path()
        .join("elsewhere")
        .join("99999999999999999999999999999999.png");
    put(&target, &thumbnail(&file_uri(&inside)));
    let link = root
        .join("normal")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png");
    symlink(&target, &link).unwrap();
    let linked_folder = root.join("fail").join("linked");
    symlink(target.parent().unwrap(), &linked_folder).unwrap();

    assert_eq!(remove_for_place(&root, &place), removed_names.len());
    for (folder, name, _) in &removed_names {
        assert!(
            !root.join(folder).join(name).exists(),
            "{folder}/{name} stayed"
        );
    }
    for (folder, name, _) in &kept_names {
        assert!(
            root.join(folder).join(name).exists(),
            "{folder}/{name} was removed"
        );
    }
    assert!(fs::symlink_metadata(&link).is_ok(), "the link was removed");
    assert!(target.exists(), "a link target was removed");
    // A second run finds nothing; the platform entry agrees.
    assert_eq!(remove_for_place(&root, &place), 0);
    assert_eq!(
        crate::vault::remove_drive_thumbnails(Some(&root), &place),
        0
    );

    // URI reading: percent-decoding, localhost, query and fragment end it.
    assert_eq!(
        file_uri_path(b"file:///a%20b/%C3%A4?x#y").unwrap(),
        Path::new("/a b/ä")
    );
    assert_eq!(
        file_uri_path(b"file://localhost/x").unwrap(),
        Path::new("/x")
    );
    assert!(file_uri_path(b"http://host/x").is_none());
    assert!(file_uri_path(b"file:relative").is_none());
    assert!(file_uri_path(b"file:///bad%zz").is_none());
}
