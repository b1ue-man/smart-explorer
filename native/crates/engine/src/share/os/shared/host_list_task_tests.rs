//! R-FS-5 (plan 2026-11-07): the Share-private names (`.se-versions`,
//! `.held.se-recycle-<16 hex>`, in every capitalisation the file operations
//! refuse) are in no listing and no hash walk, and only exactly those names.
use super::*;
use crate::{
    analytics::Progress,
    share::{
        export_config::ExportAccess,
        fs::{ShareExportConfig, SharedRoot},
        fs_host_policy_task_tests::text,
        fs_response::FsHashWalkMessage,
        wire::FsHashWalk,
    },
};
use std::sync::Arc;

/// Private under a spelling that only the Share-private rule knows (the
/// engine and stage names are exact lower case).
const PRIVATE: [&str; 3] = [
    ".SE-Versions",
    ".held.se-recycle-FEDCBA9876543210",
    ".HELD.SE-RECYCLE-0123456789ABCDEF",
];
/// Ordinary names that only look alike.
const ORDINARY: [&str; 2] = [".held.se-recycle-0123", "frei.txt"];

fn fixture() -> (tempfile::TempDir, FsAccess) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    for folder in [&root, &sub] {
        std::fs::create_dir_all(folder.join(PRIVATE[0])).unwrap();
        std::fs::write(folder.join(PRIVATE[0]).join("alt.bin"), b"old").unwrap();
        std::fs::write(folder.join(PRIVATE[1]), b"held").unwrap();
        std::fs::write(folder.join(PRIVATE[2]), b"held").unwrap();
        for name in ORDINARY {
            std::fs::write(folder.join(name), b"frei").unwrap();
        }
    }
    let access = FsAccess::dynamic(ShareExportConfig {
        roots: vec![SharedRoot::new("Docs", text(&root)).with_access(ExportAccess::ReadOnly)],
        ..Default::default()
    });
    (directory, access)
}

fn drain(mut rx: mpsc::Receiver<io::Result<FsResponse>>) -> Vec<FsResponse> {
    let mut all = Vec::new();
    while let Ok(item) = rx.try_recv() {
        all.push(item.expect("the walk is not refused"));
    }
    all
}

/// Whether `rel` has a component that is a private name.
fn names_private(rel: &str) -> bool {
    rel.split('/').any(|part| PRIVATE.contains(&part))
}

#[test]
fn review_task_private_names_are_hidden_in_listings_and_hash_walks() {
    for name in PRIVATE
        .into_iter()
        .chain([".se-versions", ".se-versions.", ".se-versions:stream"])
        .chain([".held.se-recycle-0123456789abcdef"])
    {
        assert!(hidden(name), "{name}");
    }
    for name in ORDINARY
        .into_iter()
        .chain([".held.se-recycle-0123456789abcdefg", ".se-versions-alt"])
    {
        assert!(!hidden(name), "{name}");
    }

    let (_directory, access) = fixture();
    // ListDirBatch of the top and of the subfolder.
    let top = [".held.se-recycle-0123", "frei.txt", "sub"];
    for (folder, expected) in [("/Docs", &top[..]), ("/Docs/sub", &top[..2])] {
        let (tx, rx) = mpsc::channel(64);
        let request = FsListBatch {
            path: folder.into(),
            cursor: None,
        };
        run(request, access.clone(), tx, &AtomicBool::new(false)).unwrap();
        let mut listed: Vec<String> = drain(rx)
            .into_iter()
            .flat_map(|response| match response {
                FsResponse::EntriesBatch { entries, .. } => entries,
                _ => Vec::new(),
            })
            .map(|entry| entry.name)
            .collect();
        listed.sort();
        assert_eq!(listed, expected, "{folder}");
    }

    // HashWalk over the whole export, with and without the listing attributes.
    for meta in [false, true] {
        let (tx, rx) = mpsc::channel(256);
        let request = FsHashWalk {
            path: "/Docs".into(),
            meta,
            ..Default::default()
        };
        crate::share::host_hash_walk::run(
            request,
            access.clone(),
            tx,
            Arc::new(AtomicBool::new(false)),
            Progress::default(),
        )
        .unwrap();
        let mut walked = Vec::new();
        for response in drain(rx) {
            if let FsResponse::HashWalk {
                message: FsHashWalkMessage::Batch { entries, omitted },
            } = response
            {
                walked.extend(entries.into_iter().map(|entry| entry.rel));
                walked.extend(omitted.into_iter().map(|hole| hole.rel));
            }
        }
        walked.sort();
        assert_eq!(
            walked,
            [
                ".held.se-recycle-0123",
                "frei.txt",
                "sub",
                "sub/.held.se-recycle-0123",
                "sub/frei.txt"
            ],
            "meta {meta}"
        );
        assert!(!walked.iter().any(|rel| names_private(rel)), "{walked:?}");
    }
}
