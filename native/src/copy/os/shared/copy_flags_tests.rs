use super::*;

const GIB: u64 = 1024 * 1024 * 1024;

#[test]
fn speed_task_windows_copy_flags_keep_fail_if_exists_and_copy_symlink() {
    let base = COPY_FILE_FAIL_IF_EXISTS | COPY_FILE_COPY_SYMLINK;
    for (size, memory, unc, compressed) in [
        (0, None, false, false),
        (1, Some(8 * GIB), false, false),
        (4 * GIB, Some(8 * GIB), true, true),
        (u64::MAX, Some(1), true, false),
    ] {
        let flags = windows_copy_flags(size, memory, unc, compressed);
        assert_eq!(flags & base, base, "{size} {memory:?} {unc} {compressed}");
    }
    assert_eq!(COPY_FILE_FAIL_IF_EXISTS, 1);
    assert_eq!(COPY_FILE_COPY_SYMLINK, 0x800);
    // The common case adds nothing to the two fixed flags.
    assert_eq!(windows_copy_flags(1024, Some(8 * GIB), false, false), base);
}

#[test]
fn speed_task_windows_copy_flags_no_buffering_from_a_quarter_of_available_memory() {
    let memory = 8 * GIB;
    let quarter = memory / 4;
    let has = |flags: u32| flags & COPY_FILE_NO_BUFFERING != 0;
    assert!(has(windows_copy_flags(quarter, Some(memory), false, false)));
    assert!(has(windows_copy_flags(
        quarter + 1,
        Some(memory),
        false,
        false
    )));
    assert!(!has(windows_copy_flags(
        quarter - 1,
        Some(memory),
        false,
        false
    )));
    // Unknown memory never switches the cache off.
    assert!(!has(windows_copy_flags(u64::MAX, None, false, false)));
    assert_eq!(COPY_FILE_NO_BUFFERING, 0x1000);
    // A quarter rounds down: 7 bytes of memory give a threshold of 1.
    assert!(has(windows_copy_flags(1, Some(7), false, false)));
    assert!(!has(windows_copy_flags(0, Some(7), false, false)));
}

#[test]
fn speed_task_windows_copy_flags_compressed_traffic_only_for_unc_and_when_enabled() {
    let has = |flags: u32| flags & COPY_FILE_REQUEST_COMPRESSED_TRAFFIC != 0;
    assert!(has(windows_copy_flags(1, None, true, true)));
    assert!(!has(windows_copy_flags(1, None, true, false)));
    assert!(!has(windows_copy_flags(1, None, false, true)));
    assert!(!has(windows_copy_flags(1, None, false, false)));
    assert_eq!(COPY_FILE_REQUEST_COMPRESSED_TRAFFIC, 0x1000_0000);
    // E13-S11: the switch the engine passes stays off.
    const { assert!(!WINDOWS_REQUEST_COMPRESSED_TRAFFIC) };
}

#[test]
fn speed_task_windows_is_unc_recognizes_verbatim_unc_but_not_verbatim_drive() {
    for unc in [
        r"\\server\share\file.txt",
        "//server/share/file.txt",
        r"\\?\UNC\server\share\file.txt",
        r"\\?\unc\server\share",
        "//?/UNC/server/share/x",
        r"\\.\UNC\server\share",
    ] {
        assert!(is_unc(unc), "{unc}");
    }
    for local in [
        r"C:\data\file.txt",
        "C:/data/file.txt",
        r"\\?\C:\data\file.txt",
        "//?/C:/data/file.txt",
        r"\\.\C:\data",
        r"\\?\Volume{1234}\x",
        r"\data\file.txt",
        "/home/user/file",
        "relative/path",
        "",
        r"\\",
    ] {
        assert!(!is_unc(local), "{local}");
    }
}
