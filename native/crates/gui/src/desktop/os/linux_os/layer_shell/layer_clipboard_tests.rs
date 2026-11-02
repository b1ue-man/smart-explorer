//! DSK-M7 checks of the clipboard's wire formats (no compositor): the
//! `file://` URIs, `text/uri-list` and `x-special/gnome-copied-files`.
use std::path::{Path, PathBuf};

use super::clipboard_uri::{
    file_uri, gnome_copied, parse_file_list, parse_file_uri, uri_list, Content, MIME_GNOME,
    MIME_TEXT, MIME_TEXT_UTF8, MIME_URI_LIST, MIME_UTF8_STRING,
};

fn exists(_: &Path) -> bool {
    true
}

#[test]
fn desktop_task_layer_clipboard_uri_parsing() {
    // Percent decoding, bytewise, also for UTF-8 and reserved characters.
    assert_eq!(
        parse_file_uri("file:///home/u/Bericht%20Q3.md"),
        Some(PathBuf::from("/home/u/Bericht Q3.md"))
    );
    assert_eq!(
        parse_file_uri("file:///home/u/%C3%A4%23%25%3F"),
        Some(PathBuf::from("/home/u/ä#%?"))
    );
    assert_eq!(
        parse_file_uri("  file:///tmp/a  "),
        Some(PathBuf::from("/tmp/a"))
    );
    // No host or the host localhost; any other host is another machine.
    assert_eq!(
        parse_file_uri("file://localhost/tmp/a"),
        Some(PathBuf::from("/tmp/a"))
    );
    assert_eq!(
        parse_file_uri("file://LOCALHOST/tmp/a"),
        Some(PathBuf::from("/tmp/a"))
    );
    assert_eq!(parse_file_uri("file://server/share/a"), None);
    assert_eq!(parse_file_uri("file://server"), None);
    // Not a file URI, relative, broken or with a NUL byte.
    assert_eq!(parse_file_uri("https://example.org/a"), None);
    assert_eq!(parse_file_uri("/tmp/a"), None);
    assert_eq!(parse_file_uri("file:relative/a"), None);
    assert_eq!(parse_file_uri("file://relative"), None);
    assert_eq!(parse_file_uri("file:///tmp/%zz"), None);
    assert_eq!(parse_file_uri("file:///tmp/%4"), None);
    assert_eq!(parse_file_uri("file:///tmp/a%00b"), None);
    assert_eq!(parse_file_uri(""), None);

    // A uri-list skips comments, blank lines and unusable lines, and is
    // always a copy; lines of files that do not exist are left out.
    let body = "# comment\r\nfile:///tmp/a\r\n\r\nhttps://x/y\r\nfile:///tmp/gone\r\nfile://other/b\r\nfile:///tmp/c%20d\r\n";
    let present = |path: &Path| !path.ends_with("gone");
    assert_eq!(
        parse_file_list(MIME_URI_LIST, body, present),
        Some((
            vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/c d")],
            false
        ))
    );
    // The GNOME format starts with the action.
    assert_eq!(
        parse_file_list(MIME_GNOME, "cut\nfile:///tmp/a\nfile:///tmp/b", exists),
        Some((vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")], true))
    );
    assert_eq!(
        parse_file_list(MIME_GNOME, "copy\nfile:///tmp/a", exists),
        Some((vec![PathBuf::from("/tmp/a")], false))
    );
    // Without the action word the first line is a URI already.
    assert_eq!(
        parse_file_list(MIME_GNOME, "file:///tmp/a", exists),
        Some((vec![PathBuf::from("/tmp/a")], false))
    );
    // No usable file: none.
    assert_eq!(
        parse_file_list(MIME_URI_LIST, "https://x\r\n", exists),
        None
    );
    assert_eq!(parse_file_list(MIME_GNOME, "cut\n", exists), None);
    assert_eq!(parse_file_list(MIME_URI_LIST, "", exists), None);
    assert_eq!(
        parse_file_list(MIME_URI_LIST, "file:///tmp/a", |_| false),
        None
    );
}

#[test]
fn layer_clipboard_formats_roundtrip() {
    // Everything but [A-Za-z0-9-._~/] is percent encoded, bytewise.
    assert_eq!(file_uri(Path::new("/tmp/a b")), "file:///tmp/a%20b");
    assert_eq!(file_uri(Path::new("/tmp/ä")), "file:///tmp/%C3%A4");
    assert_eq!(
        file_uri(Path::new("/tmp/x~y-z_1.txt")),
        "file:///tmp/x~y-z_1.txt"
    );
    assert_eq!(file_uri(Path::new("/tmp/100%")), "file:///tmp/100%25");

    let files = vec![
        PathBuf::from("/home/u/Bericht Q3.md"),
        PathBuf::from("/home/u/ä#%?&=+,;"),
        PathBuf::from("/home/u/Ordner/Datei"),
    ];
    // Every line ends with CRLF in the uri-list, the GNOME text joins by LF
    // without a final one.
    let list = uri_list(&files);
    assert!(list.ends_with("\r\n"));
    assert_eq!(list.matches("\r\n").count(), 3);
    assert_eq!(
        parse_file_list(MIME_URI_LIST, &list, exists),
        Some((files.clone(), false))
    );
    let gnome = gnome_copied(&files, true);
    assert!(gnome.starts_with("cut\nfile://"));
    assert!(!gnome.ends_with('\n') && !gnome.contains('\r'));
    assert_eq!(
        parse_file_list(MIME_GNOME, &gnome, exists),
        Some((files.clone(), true))
    );
    assert!(gnome_copied(&files, false).starts_with("copy\n"));

    // What a source offers and sends.
    let content = Content::Files {
        files: files.clone(),
        cut: false,
    };
    assert_eq!(
        content.mimes(),
        [MIME_URI_LIST, MIME_GNOME, MIME_TEXT_UTF8, MIME_TEXT]
    );
    assert_eq!(content.bytes_for(MIME_URI_LIST), Some(list.into_bytes()));
    assert_eq!(
        content.bytes_for(MIME_GNOME),
        Some(gnome_copied(&files, false).into_bytes())
    );
    let marker = String::from_utf8(content.bytes_for(MIME_TEXT).unwrap()).unwrap();
    assert_eq!(
        crate::app::desktop_surface_actions::parse_clip_text(&marker),
        Some((files.clone(), false)),
        "a second se surface finds the files in the text"
    );
    assert_eq!(content.bytes_for(MIME_UTF8_STRING), None);
    assert_eq!(content.bytes_for("image/png"), None);
    let text = Content::Text("Hallo".into());
    assert_eq!(text.mimes(), [MIME_TEXT_UTF8, MIME_TEXT, MIME_UTF8_STRING]);
    assert_eq!(text.bytes_for(MIME_UTF8_STRING), Some(b"Hallo".to_vec()));
    assert_eq!(text.bytes_for(MIME_URI_LIST), None);
}
