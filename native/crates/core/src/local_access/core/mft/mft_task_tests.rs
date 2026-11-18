//! MFT parser tests on synthetic record bytes; no volume, every platform.
use super::mft_task_fixtures::*;
use super::*;
use std::sync::atomic::AtomicBool;

fn scan_with(
    index: &MftIndex,
    root: FileRef,
    keep: &mut dyn FnMut(&MftEntry<'_>) -> bool,
) -> MftScan {
    index
        .build_tree(root, "C:\\", true, &AtomicBool::new(false), keep)
        .expect("tree")
}

fn scan(records: &[Rec]) -> MftScan {
    scan_with(&index(records), ROOT, &mut |_| true)
}

fn child<'a>(node: &'a MftNode, name: &str) -> &'a MftNode {
    node.children
        .iter()
        .find(|child| child.name == name)
        .unwrap_or_else(|| panic!("{name} missing in {}", node.name))
}

fn names(node: &MftNode) -> Vec<&str> {
    let mut names: Vec<&str> = node
        .children
        .iter()
        .map(|child| child.name.as_str())
        .collect();
    names.sort_unstable();
    names
}

#[test]
fn mft_task_fixup_restores_sector_ends_and_rejects_torn_writes() {
    let record = Rec::file(30)
        .named(ROOT, WIN32, "a.bin")
        .resident_data("", 600);
    let mut stored = record.build();
    assert_ne!(stored[0x38..], record.plain()[0x38..]);
    assert!(matches!(
        parse_record(&mut stored, 30),
        Ok(RecordState::InUse(_))
    ));
    assert_eq!(
        stored[0x38..],
        record.plain()[0x38..],
        "every stride end restored"
    );

    let mut torn = record.build();
    torn[1022] ^= 0xFF;
    assert_eq!(
        parse_record(&mut torn, 30),
        Err(RecordDefect::Fixup { sector: 1 })
    );
    // A freed slot does not count, torn or not.
    let mut freed = Rec::unused(31).build();
    freed[510] ^= 0xFF;
    assert_eq!(parse_record(&mut freed, 31), Ok(RecordState::Unused));
    let mut zeroed = vec![0u8; RECORD];
    assert_eq!(parse_record(&mut zeroed, 32), Ok(RecordState::Unused));
}

#[test]
fn mft_task_file_name_keeps_long_and_posix_names_and_drops_dos_alias() {
    let result = scan(&[
        root(),
        Rec::file(20)
            .named(ROOT, WIN32, "Long Name.txt")
            .named(ROOT, DOS, "LONGNA~1.TXT")
            .resident_data("", 5),
        Rec::file(21)
            .named(ROOT, POSIX, "posix:name")
            .resident_data("", 1),
    ]);
    assert_eq!(names(&result.root), ["Long Name.txt", "posix:name"]);
    assert_eq!(child(&result.root, "Long Name.txt").size, 5);
    assert!(!child(&result.root, "Long Name.txt").repeated_link);
    assert!(!result.diagnostics.partial);
}

#[test]
fn mft_task_data_sizes_resident_non_resident_compressed_and_continuation() {
    let result = scan(&[
        root(),
        Rec::file(20)
            .named(ROOT, WIN32, "resident")
            .resident_data("", 100),
        Rec::file(21)
            .named(ROOT, WIN32, "plain")
            .data("", 0, 12288, 10000, None),
        Rec::file(22)
            .named(ROOT, WIN32, "sparse")
            .data("", 0, 131072, 100000, Some(8192)),
        Rec::file(23)
            .named(ROOT, WIN32, "continued")
            .data("", 0, 8192, 8000, None)
            .data("", 2, 999_999, 999_999, None),
    ]);
    let sizes = |name| {
        let node = child(&result.root, name);
        (node.size, node.allocated, node.kind)
    };
    assert_eq!(sizes("resident"), (100, 104, MftKind::File));
    assert_eq!(sizes("plain"), (10000, 12288, MftKind::File));
    assert_eq!(sizes("sparse"), (100000, 8192, MftKind::File));
    assert_eq!(sizes("continued"), (8000, 8192, MftKind::File));
    assert_eq!(result.root.size, 100 + 10000 + 100000 + 8000);
    assert_eq!(result.root.allocated, 104 + 12288 + 8192 + 8192);
}

#[test]
fn mft_task_hard_link_counts_occupied_bytes_once() {
    let a = reference(20, 1);
    let b = reference(21, 1);
    let result = scan(&[
        root(),
        Rec::dir(20).named(ROOT, WIN32, "A"),
        Rec::dir(21).named(ROOT, WIN32, "B"),
        Rec::file(22)
            .named(a, WIN32, "x")
            .named(b, WIN32, "y")
            .data("", 0, 4096, 4000, None),
    ]);
    let x = child(child(&result.root, "A"), "x");
    let y = child(child(&result.root, "B"), "y");
    assert_eq!(
        (x.size, y.size),
        (4000, 4000),
        "every name shows its logical size"
    );
    assert_eq!(x.allocated + y.allocated, 4096, "occupied bytes once");
    assert!(x.repeated_link != y.repeated_link);
    assert_eq!(result.root.allocated, 4096);
    assert_eq!(result.root.size, 8000);
}

#[test]
fn mft_task_reparse_points_are_never_followed() {
    let result = scan(&[
        root(),
        Rec::dir(20)
            .named(ROOT, WIN32, "junction")
            .reparse(0xA000_0003),
        Rec::file(21)
            .named(reference(20, 1), WIN32, "inside")
            .data("", 0, 4096, 4096, None),
        Rec::file(22)
            .named(ROOT, WIN32, "link.txt")
            .reparse(0xA000_000C)
            .resident_data("", 50),
        Rec::file(23)
            .named(ROOT, WIN32, "wof.dll")
            .reparse(0x8000_0017)
            .data("", 0, 0, 5000, Some(0))
            .data("WofCompressedData", 0, 2048, 1900, None),
        // Reparse flag without a readable tag: treated as a link.
        Rec::file(24)
            .named(ROOT, WIN32, "unknown-tag")
            .standard(0x400)
            .resident_data("", 70),
    ]);
    let junction = child(&result.root, "junction");
    assert_eq!(junction.kind, MftKind::Link);
    assert!(junction.children.is_empty());
    assert_eq!((junction.size, junction.allocated), (0, 0));
    let link = child(&result.root, "link.txt");
    assert_eq!(
        (link.kind, link.size, link.allocated),
        (MftKind::Link, 0, 0)
    );
    assert_eq!(child(&result.root, "unknown-tag").kind, MftKind::Link);
    let wof = child(&result.root, "wof.dll");
    assert_eq!(
        (wof.kind, wof.size, wof.allocated),
        (MftKind::File, 5000, 2048)
    );
    assert_eq!(
        result.root.allocated, 2048,
        "the junction target is not counted"
    );
}

#[test]
fn mft_task_defective_records_make_the_scan_partial() {
    let clean = scan(&[
        root(),
        Rec::file(20).named(ROOT, WIN32, "ok").resident_data("", 8),
    ]);
    assert!(!clean.diagnostics.partial);

    let mut bytes = mft_bytes(&[
        root(),
        Rec::file(20).named(ROOT, WIN32, "ok").resident_data("", 8),
        Rec::file(21)
            .named(ROOT, WIN32, "torn")
            .resident_data("", 8),
        Rec::file(22).named(ROOT, WIN32, "bad").resident_data("", 8),
    ]);
    bytes[21 * RECORD + 510] ^= 0xFF;
    bytes[22 * RECORD..22 * RECORD + 4].copy_from_slice(b"BAAD");
    let index = index_from(bytes, 64 * RECORD);
    let result = scan_with(&index, ROOT, &mut |_| true);
    assert!(result.diagnostics.partial, "never silently complete");
    let notes = result.diagnostics.notes.join("\n");
    assert!(notes.contains("MFT-Datensatz 21"), "{notes}");
    assert!(
        notes.contains("MFT-Datensatz 22") && notes.contains("BAAD"),
        "{notes}"
    );
    assert_eq!(names(&result.root), ["ok"]);
}

#[test]
fn mft_task_alternate_data_streams_count_into_occupied_bytes() {
    let result = scan(&[
        root(),
        Rec::file(20)
            .named(ROOT, WIN32, "doc.txt")
            .resident_data("", 10)
            .resident_data("Zone.Identifier", 26)
            .data("big", 0, 65536, 60000, None),
    ]);
    let doc = child(&result.root, "doc.txt");
    assert_eq!(doc.size, 10, "logical size is the unnamed stream");
    assert_eq!(doc.allocated, 16 + 32 + 65536);
}

#[test]
fn mft_task_extension_records_merge_and_a_missing_one_is_partial() {
    let base = reference(20, 1);
    let merged = scan(&[
        root(),
        Rec::file(20)
            .named(ROOT, WIN32, "big.vhdx")
            .list(&[base, reference(30, 1)]),
        Rec::extension(30, base)
            .data("", 0, 1 << 20, 1_000_000, None)
            .named(ROOT, WIN32, "second name"),
    ]);
    let big = child(&merged.root, "big.vhdx");
    let second = child(&merged.root, "second name");
    assert_eq!((big.size, second.size), (1_000_000, 1_000_000));
    assert_eq!(big.allocated + second.allocated, 1 << 20);
    assert!(!merged.diagnostics.partial);

    let missing = scan(&[
        root(),
        Rec::file(21)
            .named(ROOT, WIN32, "lost.bin")
            .list(&[reference(21, 1), reference(31, 1)]),
    ]);
    assert!(missing.diagnostics.partial);
    assert!(missing.diagnostics.notes.join("\n").contains("31"));
}

#[test]
fn mft_task_system_files_form_their_own_node() {
    let extend = reference(11, 1);
    let records = [
        root(),
        Rec::file(0)
            .named(ROOT, WIN32_AND_DOS, "$MFT")
            .data("", 0, 262_144, 262_144, None),
        Rec::file(8)
            .named(ROOT, WIN32_AND_DOS, "$BadClus")
            .resident_data("", 0)
            .data("$Bad", 0, 1 << 40, 1 << 40, None),
        Rec::dir(11).named(ROOT, WIN32_AND_DOS, "$Extend"),
        Rec::file(24).named(extend, WIN32_AND_DOS, "$UsnJrnl").data(
            "$J",
            0,
            1 << 30,
            1 << 33,
            Some(1 << 20),
        ),
        Rec::dir(25).named(ROOT, WIN32, "Users"),
        Rec::file(26)
            .named(reference(25, 1), WIN32, "user.txt")
            .resident_data("", 3),
    ];
    let index = index(&records);
    let result = scan_with(&index, ROOT, &mut |_| true);
    assert_eq!(names(&result.root), ["Users"]);
    let system = result.system_files.as_ref().expect("volume root scan");
    assert_eq!(system.name, SYSTEM_FILES_NAME);
    assert_eq!(names(system), ["$BadClus", "$Extend", "$MFT"]);
    assert_eq!(child(system, "$BadClus").allocated, 0, "$Bad holds no data");
    assert_eq!(
        child(child(system, "$Extend"), "$UsnJrnl").allocated,
        1 << 20
    );
    assert_eq!(system.allocated, 262_144 + (1 << 20));

    let folder = scan_with(&index, reference(25, 1), &mut |_| true);
    assert!(folder.system_files.is_none());
    assert_eq!(names(&folder.root), ["user.txt"]);
}

#[test]
fn mft_task_records_split_across_spans_are_assembled() {
    let records = [
        root(),
        Rec::dir(20).named(ROOT, WIN32, "dir"),
        Rec::file(21)
            .named(reference(20, 1), WIN32, "a")
            .resident_data("", 700),
        Rec::file(22)
            .named(reference(20, 1), WIN32, "b")
            .data("", 0, 8192, 5000, None),
    ];
    let whole = scan_with(&index_in_spans(&records, 64 * RECORD), ROOT, &mut |_| true);
    for span in [512, 1536, 3584] {
        let split = scan_with(&index_in_spans(&records, span), ROOT, &mut |_| true);
        assert_eq!(split.root, whole.root, "span {span}");
        assert!(!split.diagnostics.partial, "span {span}");
    }
    assert_eq!(whole.root.allocated, 704 + 8192);
}

#[test]
fn mft_task_keep_callback_excludes_entries_and_subtrees() {
    let keep_dir = reference(20, 1);
    let index = index(&[
        root(),
        Rec::dir(20).named(ROOT, WIN32, "keep"),
        Rec::dir(21).named(ROOT, WIN32, ".se-staging"),
        Rec::file(22)
            .named(reference(21, 1), WIN32, "hidden")
            .data("", 0, 4096, 4096, None),
        Rec::file(23)
            .named(ROOT, WIN32, "drop.tmp")
            .data("", 0, 4096, 4096, None),
        Rec::file(24)
            .named(keep_dir, WIN32, "kept")
            .resident_data("", 9),
    ]);
    let mut seen_parents = Vec::new();
    let result = scan_with(&index, ROOT, &mut |entry| {
        if entry.name == "kept" {
            seen_parents = entry.parents.to_vec();
        }
        entry.name != ".se-staging" && entry.name != "drop.tmp"
    });
    assert_eq!(names(&result.root), ["keep"]);
    assert_eq!(seen_parents, ["keep"]);
    assert_eq!(result.root.allocated, 16);
}

#[test]
fn mft_task_directory_reached_twice_is_partial_not_endless() {
    let result = scan(&[
        root(),
        Rec::dir(20)
            .named(ROOT, WIN32, "A")
            .named(reference(21, 1), WIN32, "A-again"),
        Rec::dir(21).named(reference(20, 1), WIN32, "B"),
    ]);
    assert!(result.diagnostics.partial);
    let b = child(child(&result.root, "A"), "B");
    assert!(b.children.is_empty());
}

#[test]
fn mft_task_walk_refuses_a_stale_or_unsuitable_root() {
    let index = index(&[
        root(),
        Rec::dir(20).named(ROOT, WIN32, "dir"),
        Rec::file(21)
            .named(ROOT, WIN32, "file")
            .resident_data("", 1),
        Rec::dir(22)
            .named(ROOT, WIN32, "junction")
            .reparse(0xA000_0003),
    ]);
    let cancel = AtomicBool::new(false);
    for root in [
        reference(20, 9),
        reference(21, 1),
        reference(22, 1),
        reference(99, 1),
    ] {
        let error = index
            .build_tree(root, "x", false, &cancel, &mut |_| true)
            .expect_err("refused root");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "{root:?}");
    }
    assert_eq!(index.file_ref(20), Some(reference(20, 1)));
    let cancelled = AtomicBool::new(true);
    let error = index
        .build_tree(ROOT, "x", false, &cancelled, &mut |_| true)
        .expect_err("cancelled");
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
}

#[test]
fn mft_task_volume_layout_and_read_plan() {
    let mut raw = vec![0u8; VOLUME_DATA_LEN];
    raw[0..8].copy_from_slice(&0x55u64.to_le_bytes());
    raw[40..44].copy_from_slice(&512u32.to_le_bytes());
    raw[44..48].copy_from_slice(&4096u32.to_le_bytes());
    raw[48..52].copy_from_slice(&1024u32.to_le_bytes());
    raw[56..64].copy_from_slice(&40960i64.to_le_bytes());
    let data = parse_volume_data(&raw).expect("geometry");
    assert_eq!(data.record_count(), 40);
    let mut odd = raw.clone();
    odd[40..44].copy_from_slice(&1000u32.to_le_bytes());
    assert!(parse_volume_data(&odd).is_err());

    let mut pointers = vec![0u8; 16 + 3 * 16];
    pointers[0..4].copy_from_slice(&3u32.to_le_bytes());
    for (index, (next, lcn)) in [(4i64, 100i64), (6, -1), (12, 200)].iter().enumerate() {
        let at = 16 + index * 16;
        pointers[at..at + 8].copy_from_slice(&next.to_le_bytes());
        pointers[at + 8..at + 16].copy_from_slice(&lcn.to_le_bytes());
    }
    let (extents, next) = parse_retrieval_pointers(&pointers).expect("extents");
    assert_eq!(next, 12);
    assert_eq!(
        extents,
        [
            MftExtent {
                vcn: 0,
                lcn: Some(100),
                clusters: 4
            },
            MftExtent {
                vcn: 4,
                lcn: None,
                clusters: 2
            },
            MftExtent {
                vcn: 6,
                lcn: Some(200),
                clusters: 6
            },
        ]
    );
    let spans = plan_reads(&data, &extents, 8192).expect("plan");
    let expected = [
        (0, Some(409_600), 8192),
        (8192, Some(417_792), 8192),
        (16384, None, 8192),
        (24576, Some(819_200), 8192),
        (32768, Some(827_392), 8192),
    ];
    let got: Vec<_> = spans.iter().map(|s| (s.logical, s.disk, s.len)).collect();
    assert_eq!(got, expected);
    assert!(
        plan_reads(&data, &extents[..1], 8192).is_err(),
        "MFT not covered"
    );
}

#[test]
fn mft_task_unflushed_cache_is_noted_but_not_partial() {
    let records = [
        root(),
        Rec::file(20).named(ROOT, WIN32, "f").resident_data("", 4),
    ];
    let bytes = mft_bytes(&records);
    let mut reader = MftRecordReader::new(&volume(bytes.len() / RECORD)).expect("reader");
    reader.note_unflushed(&std::io::Error::from(std::io::ErrorKind::PermissionDenied));
    let mut bytes = bytes;
    reader.feed(0, &mut bytes).expect("feed");
    let result = scan_with(&reader.finish(), ROOT, &mut |_| true);
    assert!(!result.diagnostics.partial, "every record was read");
    assert!(result
        .diagnostics
        .notes
        .iter()
        .any(|note| note.contains(UNFLUSHED_NOTE)));
    assert_eq!(names(&result.root), ["f"]);

    let flushed = scan(&records);
    assert!(flushed.diagnostics.notes.is_empty());
}
