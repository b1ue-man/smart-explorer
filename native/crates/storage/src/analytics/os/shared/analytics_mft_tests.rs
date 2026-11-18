//! The MFT visitor on an index built from synthetic records (the MFT
//! reader's own fixtures): the walk's exclusions, hard links counted once,
//! exact sums and the system-files node of a volume root.
use super::*;
use crate::local_access::mft::{FileRef, MftIndex, MftRecordReader, MftSection, NtfsVolumeData};
use std::sync::atomic::{AtomicBool, Ordering};

#[allow(dead_code)]
#[path = "../../../../../core/src/local_access/core/mft/mft_task_fixtures.rs"]
mod fixtures;
use fixtures::{index, reference, root, Rec, ROOT, WIN32, WIN32_AND_DOS};

#[test]
fn analyse_task_mft_visitor_builds_the_walk_tree() {
    let users = reference(25, 1);
    let index = index(&[
        root(),
        Rec::file(0)
            .named(ROOT, WIN32_AND_DOS, "$MFT")
            .data("", 0, 262_144, 262_144, None),
        Rec::dir(25).named(ROOT, WIN32, "Users"),
        Rec::file(26)
            .named(users, WIN32, "a.bin")
            .data("", 0, 8192, 5000, None),
        Rec::file(27)
            .named(users, WIN32, "b.bin")
            .named(ROOT, WIN32, "b-link")
            .data("", 0, 4096, 4000, None),
        Rec::dir(28).named(ROOT, WIN32, ".se-versions"),
        Rec::file(29)
            .named(reference(28, 1), WIN32, "old.bin")
            .data("", 0, 4096, 4096, None),
        Rec::file(30)
            .named(ROOT, WIN32, "link")
            .reparse(0xA000_000C),
    ]);
    let progress = Progress::default();
    let diagnostics = Diagnostics::default();
    let budget = AnalyticsBudget::default();
    let root_path = std::path::Path::new(r"C:\");
    let mut visitor = TreeVisitor::new(root_path, &progress, &diagnostics, &budget, &[]);
    let cancel = AtomicBool::new(false);
    index
        .walk(ROOT, "C:", MftSection::Content, &cancel, &mut visitor)
        .expect("walk");
    let (mut tree, retention) = visitor.take().expect("tree");
    assert_eq!(retention, Retention::Keep);
    let mut names: Vec<&str> = tree.children.iter().map(|child| &*child.name).collect();
    names.sort_unstable();
    // No engine folder, no link, no system file in the content.
    assert_eq!(names, ["Users", "b-link"]);
    assert_eq!(tree.size, 5000 + 4000 + 4000);
    assert_eq!(tree.allocated, 8192 + 4096, "two names, one file");
    let sum = |pick: fn(&SizeNode) -> u64| tree.children.iter().map(pick).sum::<u64>();
    assert_eq!(sum(|node| node.size), tree.size);
    assert_eq!(sum(|node| node.allocated), tree.allocated);
    assert_eq!(progress.files.load(Ordering::Relaxed), 3);
    assert_eq!(progress.allocated.load(Ordering::Relaxed), 8192 + 4096);

    visitor.begin_system_files();
    index
        .walk(
            ROOT,
            SYSTEM_FILES_NAME,
            MftSection::SystemFiles,
            &cancel,
            &mut visitor,
        )
        .expect("system walk");
    let (system, retention) = visitor.take().expect("system node");
    assert_eq!(&*system.name, SYSTEM_FILES_NAME);
    assert_eq!(system.allocated, 262_144);
    attach_system_files(&mut tree, system, retention);
    assert!(tree
        .children
        .iter()
        .any(|child| &*child.name == SYSTEM_FILES_NAME));
    assert_eq!(tree.allocated, 8192 + 4096 + 262_144);

    // The way that ran is visible, a partial MFT makes the result partial.
    report(
        &diagnostics,
        root_path,
        &crate::local_access::mft::MftDiagnostics {
            partial: true,
            notes: vec!["Datensatz 77 defekt".into()],
            omitted_notes: 0,
        },
    );
    diagnostics.note(MFT_WAY);
    let outcome = diagnostics.finish(tree, false);
    assert_eq!(outcome.status, crate::analytics::ScanStatus::Partial);
    assert!(outcome.allocated_known);
    assert!(outcome.notes.iter().any(|note| note == MFT_WAY));
}
