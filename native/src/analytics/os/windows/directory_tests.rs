//! Enumeration tests: information-class fallbacks, mid-way provider failure
//! recovery, record classification.
use super::*;

#[test]
fn analytics_access_task_automatic_query_fallback_preserves_access_denial() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::write(fixture.path().join("file"), vec![0; 11]).unwrap();
    std::fs::create_dir(fixture.path().join("sub")).unwrap();
    let assert_contents = |directory: Directory| {
        let entries: Vec<_> = directory.collect::<io::Result<_>>().unwrap();
        assert_eq!(entries.len(), 2);
        let file = entries.iter().find(|entry| entry.name == "file").unwrap();
        assert_eq!(file.kind, EntryKind::File);
        assert_eq!(file.size, 11);
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.name == "sub")
                .unwrap()
                .kind,
            EntryKind::Directory
        );
    };

    let mut calls = Vec::new();
    let full =
        read_directory_with_query(fixture.path(), Layout::Extended, |file, class, buffer| {
            calls.push(class);
            if class == FileIdExtdDirectoryRestartInfo {
                Err(io::Error::from_raw_os_error(87))
            } else {
                query_directory(file, class, buffer)
            }
        })
        .unwrap();
    assert!(matches!(full.layout, Layout::Full));
    assert!(full.fallback.is_none());
    assert_contents(full);
    // The listing continues with the full class until the provider reports
    // the end; the extended class is never asked again.
    assert_eq!(
        calls[..2],
        [FileIdExtdDirectoryRestartInfo, FileFullDirectoryRestartInfo]
    );
    assert!(calls[2..]
        .iter()
        .all(|class| *class == FileFullDirectoryInfo));

    let mut calls = Vec::new();
    let ordinary = read_directory_with_query(fixture.path(), Layout::Extended, |_, class, _| {
        calls.push(class);
        Err(io::Error::from_raw_os_error(50))
    })
    .unwrap();
    assert!(ordinary.fallback.is_some());
    assert_contents(ordinary);
    assert_eq!(
        calls,
        [FileIdExtdDirectoryRestartInfo, FileFullDirectoryRestartInfo]
    );

    // A denial at either query boundary is not an unsupported-class signal.
    // The ordinary listing above proves a wrong fallback would hide it.
    for deny_full in [false, true] {
        let mut calls = Vec::new();
        let result = read_directory_with_query(fixture.path(), Layout::Extended, |_, class, _| {
            calls.push(class);
            let code = if deny_full && class == FileIdExtdDirectoryRestartInfo {
                87
            } else {
                5
            };
            Err(io::Error::from_raw_os_error(code))
        });
        let error = result
            .err()
            .expect("query denial must fail directory startup");
        assert_eq!(error.raw_os_error(), Some(5));
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let expected = if deny_full {
            vec![FileIdExtdDirectoryRestartInfo, FileFullDirectoryRestartInfo]
        } else {
            vec![FileIdExtdDirectoryRestartInfo]
        };
        assert_eq!(calls, expected);
    }
}

#[test]
fn analytics_access_task_midway_query_failure_finishes_through_ordinary_listing() {
    let fixture = tempfile::tempdir().unwrap();
    for index in 0..40 {
        std::fs::write(fixture.path().join(format!("file{index:02}")), vec![0; 3]).unwrap();
    }
    let mut calls = 0usize;
    let directory =
        read_directory_with_query(fixture.path(), Layout::Extended, |file, class, buffer| {
            calls += 1;
            if calls == 1 {
                // Deliver only the first batch through a tiny window, then
                // fail the continuation like a flaky provider would.
                query_directory(file, class, &mut buffer[..64])
            } else {
                Err(io::Error::from_raw_os_error(1117))
            }
        })
        .unwrap();
    let mut names = Vec::new();
    let mut errors = 0;
    for entry in directory {
        match entry {
            Ok(entry) => names.push(entry.name),
            Err(_) => errors += 1,
        }
    }
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 40, "every entry is yielded exactly once");
    assert_eq!(errors, 0);
    assert!(
        calls >= 2,
        "the failing continuation must have been attempted"
    );
}

#[test]
fn analytics_access_task_full_record_fallback_and_reparse_classification() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::write(fixture.path().join("file"), vec![0; 11]).unwrap();
    std::fs::create_dir(fixture.path().join("sub")).unwrap();
    let full = read_directory_with_layout(fixture.path(), Layout::Full).unwrap();
    let entries: Vec<_> = full.collect::<io::Result<_>>().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries
            .iter()
            .find(|entry| entry.name == "file")
            .unwrap()
            .size,
        11
    );
    assert_eq!(
        kind(
            FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT,
            0xa0000003
        ),
        EntryKind::Link
    );
    assert_eq!(
        kind(
            FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT,
            0xa000000c
        ),
        EntryKind::Link
    );
    assert_eq!(
        kind(
            FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT,
            0x9000001a
        ),
        EntryKind::Directory
    );
    for code in [1, 50, 87, 124] {
        assert!(unsupported(&io::Error::from_raw_os_error(code)));
    }
    for code in [5, 32, 18, 1117] {
        assert!(!unsupported(&io::Error::from_raw_os_error(code)));
    }
    assert!(enumeration_ended(&io::Error::from_raw_os_error(
        ERROR_NO_MORE_FILES as i32
    )));
    assert!(enumeration_ended(&io::Error::from_raw_os_error(
        ERROR_HANDLE_EOF as i32
    )));
    // An unreadable reparse tag never drops the entry.
    assert_eq!(reparse_tag(Path::new(r"\\?\C:\definitely\missing\path")), 0);
}
