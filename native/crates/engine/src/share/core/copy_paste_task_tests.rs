use std::fs;
use std::io::{self, Read, Write};

use super::copy_paste_task_fixture::CopyPastePeerFixture;
use super::wire::{Ctrl, FsErrorKind, FsResponse};

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_share_cross_export_nested_unicode_and_empty() -> io::Result<()> {
    assert!(CopyPastePeerFixture::enabled(), "requires isolated remote task runner");
    let fixture = CopyPastePeerFixture::new()?;
    let backend = &fixture.backend;
    backend.mkdir_all("/A/nested/Gr\u{fc}\u{df}e")?;
    backend.mkdir_all("/B/nested/Gr\u{fc}\u{df}e")?;
    let source = "/A/nested/Gr\u{fc}\u{df}e/\u{6587}\u{4ef6}.txt";
    let destination = "/B/nested/Gr\u{fc}\u{df}e/\u{6587}\u{4ef6}.txt";
    let payload = "authenticated Share copy: Gr\u{fc}\u{df}e \u{6587}\u{4ef6}".as_bytes();
    {
        let mut writer = backend.open_write(source)?;
        writer.write_all(payload)?;
        writer.flush()?;
    }
    assert_eq!(backend.copy_file(source, destination)?, payload.len() as u64);
    let relative = "nested/Gr\u{fc}\u{df}e/\u{6587}\u{4ef6}.txt";
    assert_eq!(fs::read(fixture.root_a.join(relative))?, payload);
    assert_eq!(fs::read(fixture.root_b.join(relative))?, payload);
    let mut received = Vec::new();
    backend.open_read(destination)?.read_to_end(&mut received)?;
    assert_eq!(received, payload);

    // CopyFile retains its existing replacing contract. GUI no-replace copy
    // separately promotes its private final stage in the application coverage.
    fs::write(fixture.root_b.join(relative), b"previous destination")?;
    assert_eq!(backend.copy_file(source, destination)?, payload.len() as u64);
    assert_eq!(fs::read(fixture.root_b.join(relative))?, payload);
    backend.open_write("/A/empty.bin")?.flush()?;
    assert_eq!(backend.copy_file("/A/empty.bin", "/B/empty.bin")?, 0);
    assert_eq!(fs::metadata(fixture.root_b.join("empty.bin"))?.len(), 0);
    assert_eq!(fs::metadata(fixture.root_a.join("empty.bin"))?.len(), 0);
    Ok(())
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_share_rename_conflict_and_export_authority() -> io::Result<()> {
    assert!(CopyPastePeerFixture::enabled(), "requires isolated remote task runner");
    let fixture = CopyPastePeerFixture::new()?;
    let backend = &fixture.backend;
    fs::write(fixture.root_a.join("source.txt"), b"preserve source")?;
    backend.rename("/A/source.txt", "/A/renamed.txt")?;
    assert!(!fixture.root_a.join("source.txt").exists());
    assert!(backend.rename("/A/renamed.txt", "/B/moved.txt").is_err());
    assert!(backend.promote_staged("/A/renamed.txt", "/B/promoted.txt").is_err());
    assert!(backend.copy_file("/A/renamed.txt", "/Unexported/copied.txt").is_err());
    assert!(backend.mkdir_all("/A/../escape").is_err());
    assert!(backend.open_write("/synthetic-root-file.txt").is_err());
    assert!(!fixture.root_b.join("moved.txt").exists());
    assert!(!fixture.root_b.join("promoted.txt").exists());
    let conflict = match backend.open_write_new("/A/renamed.txt") {
        Ok(_) => panic!("exclusive Share writer adopted an existing file"),
        Err(error) => error,
    };
    assert_eq!(conflict.kind(), io::ErrorKind::AlreadyExists);
    fixture.revoke_access()?;
    assert!(backend.copy_file("/A/renamed.txt", "/B/unauthorized.txt").is_err());
    assert!(backend.mkdir_all("/B/unauthorized-directory").is_err());
    assert!(!fixture.root_b.join("unauthorized.txt").exists());
    assert!(!fixture.root_b.join("unauthorized-directory").exists());
    assert_eq!(fs::read(fixture.root_a.join("renamed.txt"))?, b"preserve source");
    Ok(())
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_share_operation_whitespace_is_exact() -> io::Result<()> {
    assert!(CopyPastePeerFixture::enabled(), "requires isolated remote task runner");
    assert_eq!(super::fs_paths::split_clean("/A/ report \t")?, vec!["A", " report \t"]);
    assert_eq!(super::fs_paths::split_clean("/A/\u{2003}name\u{2003}")?, vec!["A", "\u{2003}name\u{2003}"]);
    assert_eq!(super::fs_paths::split_clean("/A/   ")?, vec!["A", "   "]);
    assert_eq!(super::fs_paths::split_clean("/A/report /child ")?, vec!["A", "report ", "child "]);
    assert!(super::fs_paths::split_clean("/A/..").is_err());
    assert!(super::fs_paths::split_clean("/A/bad\\name").is_err());
    assert!(super::fs_paths::split_clean("/A/bad\0name").is_err());
    Ok(())
}

#[test]
#[ignore = "requires isolated remote copy/paste task runner"]
fn copy_paste_task_share_typed_errors_keep_legacy_compatibility() -> io::Result<()> {
    assert!(CopyPastePeerFixture::enabled(), "requires isolated remote task runner");
    for kind in [io::ErrorKind::AlreadyExists, io::ErrorKind::Unsupported,
        io::ErrorKind::PermissionDenied, io::ErrorKind::NotFound]
    {
        let encoded = serde_json::to_vec(&Ctrl::FsResp {
            resp: super::fs_error::response(&io::Error::new(kind, "fixture detail")),
        }).map_err(io::Error::other)?;
        let decoded: Ctrl = serde_json::from_slice(&encoded).map_err(io::Error::other)?;
        let Ctrl::FsResp { resp } = decoded else { panic!("filesystem response expected"); };
        let error = super::framing::decode_resp(resp).unwrap_err();
        assert_eq!(error.kind(), kind);
        assert_eq!(error.to_string(), "fixture detail");
    }
    for json in [r#"{"r":"err","msg":"legacy"}"#,
        r#"{"r":"err","kind":"future_kind","msg":"legacy"}"#]
    {
        let decoded: FsResponse = serde_json::from_str(json).map_err(io::Error::other)?;
        let error = super::framing::decode_resp(decoded).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(error.to_string(), "legacy");
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum LegacyKind { NotFound, PermissionDenied, #[serde(other)] Unknown }
    for kind in [FsErrorKind::AlreadyExists, FsErrorKind::Unsupported] {
        let json = serde_json::to_string(&kind).map_err(io::Error::other)?;
        assert!(matches!(serde_json::from_str::<LegacyKind>(&json).map_err(io::Error::other)?,
            LegacyKind::Unknown));
    }
    Ok(())
}
