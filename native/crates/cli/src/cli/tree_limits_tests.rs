//! Destinations that cannot store every entry (U7, U11): the plan leaves out
//! and reports each refused entry with its reason and copies the rest; the
//! engine path refuses a single root and hands a tree with such entries to
//! the plan before it creates anything.
use super::TargetRules;
use crate::cli::engine_copy::try_copy_with;
use crate::cli::target::Target;
use crate::cli::tree_observer::TreeObserver;
use crate::cli::tree_ops::{copy_entry, remove_copied_source, TreeOutcome};
use crate::vfs::{
    Backend, BackendExtensions, BackendHandle, CaseFolding, LocalBackend, NameLimit, NameRules,
    Scheme, TargetLimits, VfsMeta, VfsResult,
};
use std::io::{Read, Write};
use std::sync::Arc;

/// Largest file the fake destination stores (a FAT limit in small).
const LIMIT: u64 = 4;

/// A local folder behind a remote-like backend whose target limits are
/// those of FAT with a 4-byte file limit.
struct SmallFat;

impl Backend for SmallFat {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        LocalBackend::new("/").list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        LocalBackend::new("/").stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        LocalBackend::new("/").open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        LocalBackend::new("/").open_write(path)
    }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        LocalBackend::new("/").open_write_new(path)
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        LocalBackend::new("/").rename(src, dst)
    }
    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        LocalBackend::new("/").rename_no_replace(src, dst)
    }
    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        LocalBackend::new("/").promote_staged(staged, destination)
    }
    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        LocalBackend::new("/").promote_staged_no_replace(staged, destination)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        LocalBackend::new("/").remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        LocalBackend::new("/").remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        LocalBackend::new("/").mkdir_all(path)
    }
    fn create_dir(&self, path: &str) -> VfsResult<()> {
        LocalBackend::new("/").create_dir(path)
    }
}

impl BackendExtensions for SmallFat {
    fn target_limits(&self, _root: &str) -> TargetLimits {
        TargetLimits {
            windows_names: true,
            max_name: Some(NameLimit::Utf16Units(255)),
            max_file_size: Some(LIMIT),
            ..TargetLimits::default()
        }
    }
}

fn text(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn local(path: &std::path::Path) -> Target {
    let backend: BackendHandle = Arc::new(LocalBackend::new("/"));
    Target::with_backend_key(backend, text(path), "local")
}

fn small_fat(path: &std::path::Path) -> Target {
    let backend: BackendHandle = Arc::new(SmallFat);
    Target::with_backend_key(backend, text(path), "small-fat")
}

/// `ok.txt` (3 bytes), `big.bin` (10 bytes), `sub/inner.txt`, and on Unix a
/// name Windows rules forbid.
fn source_tree(root: &std::path::Path) -> std::path::PathBuf {
    let source = root.join("source");
    std::fs::create_dir_all(source.join("sub")).expect("source");
    std::fs::write(source.join("ok.txt"), b"abc").expect("ok");
    std::fs::write(source.join("big.bin"), b"0123456789").expect("big");
    std::fs::write(source.join("sub").join("inner.txt"), b"in").expect("inner");
    #[cfg(unix)]
    std::fs::write(source.join("bad:name.txt"), b"x").expect("bad name");
    source
}

fn meta(name: &str) -> VfsMeta {
    VfsMeta {
        name: name.to_string(),
        size: 1,
        ..VfsMeta::default()
    }
}

#[test]
fn wechselmedium_task_plan_reports_refused_entries_and_copies_the_rest() {
    let scratch = tempfile::tempdir().expect("scratch");
    let source = source_tree(scratch.path());
    let destination = scratch.path().join("stick");
    let receipt = copy_entry(
        &LocalBackend::new("/"),
        &text(&source),
        &SmallFat,
        &text(&destination),
        true,
        false,
    )
    .expect("the storable entries are copied");
    assert_eq!(
        std::fs::read(destination.join("ok.txt")).expect("ok"),
        b"abc"
    );
    assert_eq!(
        std::fs::read(destination.join("sub").join("inner.txt")).expect("inner"),
        b"in"
    );
    assert!(!destination.join("big.bin").exists(), "nothing was written");
    let outcome = TreeOutcome {
        destination: text(&destination),
        omitted: receipt.omissions(),
        kept: Vec::new(),
        update: None,
    };
    let big = format!("{}/big.bin", text(&source));
    let reason = outcome
        .omitted
        .iter()
        .find(|omission| omission.path == big)
        .and_then(|omission| omission.reason.clone())
        .expect("the large file is reported");
    assert_eq!(
        reason,
        "größer als 4 Byte: das Ziel-Dateisystem kann sie nicht speichern"
    );
    assert!(outcome
        .notices()
        .iter()
        .any(|line| line == &format!("nicht übertragen: {big}: {reason}")));
    #[cfg(unix)]
    {
        assert!(!destination.join("bad:name.txt").exists());
        let bad = format!("{}/bad:name.txt", text(&source));
        let reason = outcome
            .omitted
            .iter()
            .find(|omission| omission.path == bad)
            .and_then(|omission| omission.reason.clone())
            .expect("the forbidden name is reported");
        assert!(
            reason.starts_with("Name nicht erlaubt auf dem Ziel-Dateisystem: "),
            "{reason}"
        );
    }
    // A move keeps every refused file and the folders holding them.
    let kept = remove_copied_source(&LocalBackend::new("/"), &receipt).expect("cleanup");
    assert_eq!(kept, [text(&source)]);
    assert!(source.join("big.bin").exists());
    assert!(!source.join("ok.txt").exists());
    assert!(!source.join("sub").exists());
}

#[test]
fn wechselmedium_task_engine_path_refuses_a_root_and_hands_trees_to_the_plan() {
    let scratch = tempfile::tempdir().expect("scratch");
    let source = source_tree(scratch.path());
    let observer = TreeObserver::default();
    let big = local(&source.join("big.bin"));
    let big_meta = big.backend.stat(&big.path).expect("stat");
    let copy = scratch.path().join("copy.bin");
    let error = try_copy_with(
        true,
        &big,
        &big_meta,
        &big_meta,
        &small_fat(&copy),
        &text(&copy),
        false,
        &observer,
    )
    .expect_err("a root the destination cannot store fails with its reason");
    assert!(error.ends_with("größer als 4 Byte: das Ziel-Dateisystem kann sie nicht speichern"));
    assert!(!copy.exists());

    let tree = local(&source);
    let tree_meta = tree.backend.stat(&tree.path).expect("stat");
    let stick = scratch.path().join("stick");
    let outcome = try_copy_with(
        true,
        &tree,
        &tree_meta,
        &tree_meta,
        &small_fat(&stick),
        &text(&stick),
        true,
        &observer,
    )
    .expect("no error");
    assert!(outcome.is_none(), "the plan reports each refused entry");
    assert!(!stick.exists(), "nothing was created");
}

#[test]
fn wechselmedium_task_folding_target_refuses_the_second_spelling() {
    let rules = TargetRules {
        limits: TargetLimits::default(),
        names: NameRules {
            label: Some("FAT"),
            folding: CaseFolding::Ascii,
        },
    };
    let children = [meta("a.txt"), meta("A.txt"), meta("b.txt")];
    let refused = rules.refusals(&children);
    assert_eq!(refused.len(), 1);
    let reason = refused
        .get(&0)
        .expect("byte order keeps A.txt, refuses a.txt");
    assert!(
        reason.contains("„a.txt“") && reason.contains("„A.txt“") && reason.contains("FAT"),
        "{reason}"
    );
    // Without folding both spellings are separate names.
    let plain = TargetRules::default();
    assert!(plain.refusals(&children).is_empty());
    // Links are never written, so never refused.
    let mut link = meta("a:b");
    link.is_symlink = true;
    let windows = TargetRules {
        limits: TargetLimits {
            windows_names: true,
            ..TargetLimits::default()
        },
        names: NameRules::default(),
    };
    assert_eq!(windows.refusal("a:b", &link), None);
    assert!(windows.refusal("a:b", &meta("a:b")).is_some());
    assert!(windows.may_refuse_from(&TargetRules::default()));
    assert!(!windows.may_refuse_from(&windows), "NTFS to NTFS");
}
