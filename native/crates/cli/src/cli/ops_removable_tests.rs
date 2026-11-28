//! `se cp`/`se mv` on real FAT and exFAT volumes (U4, U7, U10). These need
//! writable folders on such volumes and run in Suite crates-task:
//! Linux `SE_TEST_VFAT_DIR` and `SE_TEST_EXFAT_DIR`, Windows
//! `SE_TEST_FAT32_DIR` and `SE_TEST_EXFAT_DIR`.
use crate::cli::folder_conflict::CommandContext;
use crate::cli::target::Target;
use crate::cli::tree_observer::TreeObserver;
use crate::vfs::{BackendHandle, LocalBackend};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(not(windows))]
const FAT_DIR: &str = "SE_TEST_VFAT_DIR";
#[cfg(windows)]
const FAT_DIR: &str = "SE_TEST_FAT32_DIR";
const EXFAT_DIR: &str = "SE_TEST_EXFAT_DIR";

/// A fresh folder on the volume the variable names, removed afterwards.
struct OnVolume(PathBuf);

impl OnVolume {
    fn new(variable: &str, tag: &str) -> Self {
        let base = std::env::var_os(variable)
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                panic!("{variable} must name a writable folder on that volume (Suite crates-task)")
            });
        let unique = format!(
            "se-wechselmedium-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        );
        let path = base.join(unique);
        std::fs::create_dir_all(&path).expect("scratch folder on the volume");
        Self(path)
    }
}

impl Drop for OnVolume {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn text(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn local(path: &Path) -> Target {
    let backend: BackendHandle = Arc::new(LocalBackend::new("/"));
    Target::with_backend_key(backend, text(path), "local")
}

fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .expect("list")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
#[ignore = "needs FAT and exFAT volumes (SE_TEST_VFAT_DIR/SE_TEST_FAT32_DIR, SE_TEST_EXFAT_DIR); Suite crates-task"]
fn wechselmedium_task_interrupted_copy_resumes_on_fat_and_exfat() {
    for variable in [FAT_DIR, EXFAT_DIR] {
        let volume = OnVolume::new(variable, "resume");
        let source = volume.0.join("Quelle");
        std::fs::create_dir_all(source.join("sub")).expect("source");
        std::fs::write(source.join("a.txt"), b"a").expect("a");
        std::fs::write(source.join("sub").join("b.txt"), b"b").expect("b");
        let parent = volume.0.join("Ziel");
        std::fs::create_dir(&parent).expect("destination folder");
        let (src, dst) = (local(&source), local(&parent));
        assert_eq!(
            crate::vfs::directory_identity(&*dst.backend, &dst.path).expect("identity"),
            None,
            "{variable}: the premise, no stable folder id"
        );
        let location = crate::cli::folder_conflict::location_identity(&*dst.backend, &dst.path)
            .expect("the volume has a serial or UUID");
        assert!(location.starts_with("volume-location:v1:"), "{location}");
        // A job that was begun and never finished, then resumed.
        let (meta, destination) =
            crate::cli::transfer::transfer_plan(&src, &dst).expect("transfer plan");
        let id = CommandContext::begin(
            &src,
            &dst,
            &meta,
            destination,
            true,
            false,
            false,
            None,
            None,
        )
        .expect("begin")
        .record
        .id
        .clone();
        super::copy_with_folder_stats(&src, &dst, true, false, false, None, Some(&id))
            .unwrap_or_else(|error| panic!("{variable}: resume: {error}"));
        assert_eq!(
            std::fs::read(parent.join("Quelle").join("sub").join("b.txt")).expect("copied"),
            b"b"
        );
    }
}

#[test]
#[ignore = "needs a FAT volume (SE_TEST_VFAT_DIR on Linux, SE_TEST_FAT32_DIR on Windows); Suite crates-task"]
fn wechselmedium_task_file_over_4_gib_is_reported_and_the_rest_copied() {
    let volume = OnVolume::new(FAT_DIR, "limit");
    let scratch = tempfile::tempdir().expect("source folder");
    let source = scratch.path().join("Urlaub");
    std::fs::create_dir(&source).expect("source");
    std::fs::write(source.join("klein.txt"), b"klein").expect("small file");
    // Sparse on Linux; Windows reserves the size without writing it. It is
    // never read: the plan refuses it from its size.
    std::fs::File::create(source.join("film.mkv"))
        .and_then(|file| file.set_len(4 << 30))
        .expect("large source");
    let stick = volume.0.join("Urlaub");
    let observer = TreeObserver::default();
    let outcome = super::copy_observed(&local(&source), &local(&stick), true, false, &observer)
        .expect("the storable files are copied");
    assert_eq!(
        std::fs::read(stick.join("klein.txt")).expect("copied"),
        b"klein"
    );
    assert!(!stick.join("film.mkv").exists());
    let film = format!("{}/film.mkv", text(&source));
    let reason = outcome
        .omitted
        .iter()
        .find(|omission| omission.path == film)
        .and_then(|omission| omission.reason.clone())
        .expect("the large file is reported");
    assert!(
        reason.starts_with("größer als 4 GiB − 1 Byte: FAT")
            && reason.ends_with("kann sie nicht speichern"),
        "{reason}"
    );
    // A single such file fails with that reason before anything is written.
    let alone = volume.0.join("film.mkv");
    let error = super::copy_observed(
        &local(&source.join("film.mkv")),
        &local(&alone),
        false,
        false,
        &observer,
    )
    .expect_err("the file cannot be stored");
    assert!(error.contains("größer als 4 GiB − 1 Byte"), "{error}");
    assert_eq!(
        names(&volume.0),
        ["Urlaub"],
        "no stage or partial file is left"
    );
}

#[test]
#[ignore = "needs FAT and exFAT volumes (SE_TEST_VFAT_DIR/SE_TEST_FAT32_DIR, SE_TEST_EXFAT_DIR); Suite crates-task"]
fn wechselmedium_task_move_changes_only_the_case_on_fat_and_exfat() {
    for variable in [FAT_DIR, EXFAT_DIR] {
        let volume = OnVolume::new(variable, "case");
        std::fs::write(volume.0.join("foto.jpg"), b"bild").expect("file");
        std::fs::create_dir(volume.0.join("fotos")).expect("folder");
        std::fs::write(volume.0.join("fotos").join("x.txt"), b"x").expect("inner");
        super::move_path(
            &local(&volume.0.join("foto.jpg")),
            &local(&volume.0.join("Foto.jpg")),
            false,
            false,
        )
        .unwrap_or_else(|error| panic!("{variable}: file: {error}"));
        super::move_path(
            &local(&volume.0.join("fotos")),
            &local(&volume.0.join("Fotos")),
            true,
            false,
        )
        .unwrap_or_else(|error| panic!("{variable}: folder: {error}"));
        assert_eq!(names(&volume.0), ["Foto.jpg", "Fotos"], "{variable}");
        assert_eq!(
            std::fs::read(volume.0.join("Foto.jpg")).expect("file"),
            b"bild"
        );
        assert_eq!(names(&volume.0.join("Fotos")), ["x.txt"]);
    }
}
