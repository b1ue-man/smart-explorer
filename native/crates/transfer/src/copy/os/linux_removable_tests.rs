//! Removable media (2026-11-27): the direct removal of the app's own entry
//! (U1), the restore without replacement through the held parent (U3), and
//! moves, copies and their cancellation on real FAT and exFAT volumes.
use super::*;
use crate::copy::{transfer_local, LocalOutcome, LocalRequest};
use crate::types::{Conflict, CopyMode};
use crate::vfs::{Backend, LocalBackend};
use std::sync::atomic::{AtomicBool, Ordering};

/// U1: the direct removal deletes only the held object; a name that was
/// swapped meanwhile is refused and kept.
#[test]
fn removable_task_own_entry_removal_checks_the_held_object() {
    let fixture = tempfile::tempdir().unwrap();
    let stage = fixture
        .path()
        .join(".a.smart-explorer-0123456789abcdef.part");
    std::fs::write(&stage, b"stage").unwrap();
    let parent = ConfinedDirectory::open_root(fixture.path()).unwrap();
    let held = File::open(&stage).unwrap();
    let name = stage.file_name().unwrap();

    std::fs::rename(&stage, fixture.path().join("elsewhere")).unwrap();
    std::fs::write(&stage, b"someone else").unwrap();
    let error = remove_own_file(&parent, name, &held).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
    assert_eq!(std::fs::read(&stage).unwrap(), b"someone else");

    std::fs::remove_file(&stage).unwrap();
    std::fs::rename(fixture.path().join("elsewhere"), &stage).unwrap();
    remove_own_file(&parent, name, &held).unwrap();
    assert!(!stage.exists());
}

/// U3: the fallback renames below the held parent even after its path was
/// renamed, and never replaces a taken name.
#[test]
fn removable_task_restore_through_held_parent_never_replaces() {
    let fixture = tempfile::tempdir().unwrap();
    let folder = fixture.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    let quarantine = OsStr::new(".a.smart-explorer-0123456789abcdef.move");
    std::fs::write(folder.join(quarantine), b"source").unwrap();
    let parent = ConfinedDirectory::open_root(&folder).unwrap();
    let moved = fixture.path().join("moved");
    std::fs::rename(&folder, &moved).unwrap();

    rename_through_anchor(&parent, quarantine, OsStr::new("a")).unwrap();
    assert_eq!(std::fs::read(moved.join("a")).unwrap(), b"source");
    assert!(!moved.join(quarantine).exists());

    std::fs::write(moved.join(quarantine), b"second").unwrap();
    let error = rename_through_anchor(&parent, quarantine, OsStr::new("a")).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(std::fs::read(moved.join("a")).unwrap(), b"source");
    assert_eq!(std::fs::read(moved.join(quarantine)).unwrap(), b"second");
}

/// The directories of the suite's FAT and exFAT images, mounted through a
/// loop device as udisks2 mounts a stick: `uid=`/`gid=` of the test user,
/// `dmask=0022,fmask=0133` (new folders show 0755, files 0644).
fn mount_mode_volumes() -> Vec<PathBuf> {
    ["SE_TEST_VFAT_DIR", "SE_TEST_EXFAT_DIR"]
        .iter()
        .map(|name| {
            PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is not set")))
        })
        .collect()
}

/// A fresh folder of this test on `volume`; a volume that shows private
/// modes would not reproduce the removable-media case.
fn fresh_folder(volume: &Path, label: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let folder = volume.join(format!("se-transfer-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir(&folder).unwrap();
    let mode = std::fs::metadata(&folder).unwrap().permissions().mode();
    assert_ne!(
        mode & 0o077,
        0,
        "{}: mount with dmask=0022 (mode {mode:o})",
        volume.display()
    );
    folder
}

fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Bytes that differ from block to block, so a copy is checked byte for byte.
fn content(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
}

fn transfer(
    source: &Path,
    target: &Path,
    mode: CopyMode,
    cancel_at_first_progress: bool,
) -> LocalOutcome {
    let cancel = AtomicBool::new(false);
    let request = LocalRequest {
        source,
        target,
        conflict: Conflict::Skip,
        mode,
        cancel: &cancel,
        // Ranges are asked for: on FAT/exFAT the stage is still filled by one
        // stream (U6).
        parallel: 4,
        probe: None,
    };
    let mut progress = |_: u64| {
        if cancel_at_first_progress {
            cancel.store(true, Ordering::Release);
        }
    };
    transfer_local(&request, &mut progress)
        .unwrap_or_else(|failure| panic!("{source:?} -> {target:?}: {failure:?}"))
}

/// U1: a move onto the volume and one off it leave neither the
/// `.<name>.smart-explorer-<hex>.move` quarantine nor a
/// `.held.se-recycle-<hex>` folder nor a stage; the bytes arrive intact.
#[test]
#[ignore = "needs a mounted vfat (SE_TEST_VFAT_DIR) and exFAT (SE_TEST_EXFAT_DIR) directory; Suite crates-task"]
fn removable_task_moves_to_and_from_fat_and_exfat_leave_nothing() {
    let bytes = content(3 * 1024 * 1024 + 17);
    for volume in mount_mode_volumes() {
        let local = tempfile::tempdir().unwrap();
        let folder = fresh_folder(&volume, "move");
        std::fs::write(local.path().join("a.bin"), &bytes).unwrap();

        let outcome = transfer(
            &local.path().join("a.bin"),
            &folder.join("a.bin"),
            CopyMode::Move,
            false,
        );
        assert!(
            matches!(outcome, LocalOutcome::Completed { .. }),
            "{outcome:?}"
        );
        assert_eq!(names(&folder), ["a.bin"], "{}", volume.display());
        assert!(names(local.path()).is_empty());
        assert_eq!(std::fs::read(folder.join("a.bin")).unwrap(), bytes);

        let outcome = transfer(
            &folder.join("a.bin"),
            &local.path().join("b.bin"),
            CopyMode::Move,
            false,
        );
        assert!(
            matches!(outcome, LocalOutcome::Completed { .. }),
            "{outcome:?}"
        );
        assert!(
            names(&folder).is_empty(),
            "{}: {:?}",
            volume.display(),
            names(&folder)
        );
        assert_eq!(names(local.path()), ["b.bin"]);
        assert_eq!(std::fs::read(local.path().join("b.bin")).unwrap(), bytes);
        std::fs::remove_dir_all(&folder).unwrap();
    }
}

/// U1 and U3: a canceled copy onto the volume leaves no stage; a canceled
/// move off it puts the source back under its own name and leaves no
/// quarantine.
#[test]
#[ignore = "needs a mounted vfat (SE_TEST_VFAT_DIR) and exFAT (SE_TEST_EXFAT_DIR) directory; Suite crates-task"]
fn removable_task_canceled_transfers_on_fat_and_exfat_clean_up() {
    let bytes = content(40 * 1024 * 1024);
    for volume in mount_mode_volumes() {
        let local = tempfile::tempdir().unwrap();
        let folder = fresh_folder(&volume, "cancel");
        std::fs::write(local.path().join("big.bin"), &bytes).unwrap();

        let outcome = transfer(
            &local.path().join("big.bin"),
            &folder.join("big.bin"),
            CopyMode::Copy,
            true,
        );
        assert!(matches!(outcome, LocalOutcome::Canceled), "{outcome:?}");
        assert!(
            names(&folder).is_empty(),
            "{}: {:?}",
            volume.display(),
            names(&folder)
        );

        std::fs::write(folder.join("big.bin"), &bytes).unwrap();
        let outcome = transfer(
            &folder.join("big.bin"),
            &local.path().join("out.bin"),
            CopyMode::Move,
            true,
        );
        assert!(matches!(outcome, LocalOutcome::Canceled), "{outcome:?}");
        assert_eq!(names(&folder), ["big.bin"], "{}", volume.display());
        assert_eq!(names(local.path()), ["big.bin"]);
        assert_eq!(std::fs::read(folder.join("big.bin")).unwrap(), bytes);
        std::fs::remove_dir_all(&folder).unwrap();
    }
}

/// U2: `stat` of `fotos` reports the listed `Fotos` (the name domain the
/// transfer preflight compares with the listing), a file copied into
/// `fotos/` lands in the existing `Fotos/`, and no second folder appears.
#[test]
#[ignore = "needs a mounted vfat (SE_TEST_VFAT_DIR) and exFAT (SE_TEST_EXFAT_DIR) directory; Suite crates-task"]
fn removable_task_copy_into_existing_folder_of_other_spelling() {
    for volume in mount_mode_volumes() {
        let local = tempfile::tempdir().unwrap();
        let folder = fresh_folder(&volume, "spelling");
        std::fs::create_dir(folder.join("Fotos")).unwrap();
        let backend = LocalBackend::new("/");
        let asked = folder.join("fotos");
        let stat = backend.stat(asked.to_str().unwrap()).unwrap();
        assert_eq!(stat.name, "Fotos", "{}", volume.display());
        let listed = backend.list_dir(folder.to_str().unwrap()).unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|meta| meta.name.as_str())
                .collect::<Vec<_>>(),
            ["Fotos"]
        );
        std::fs::write(local.path().join("bild.jpg"), b"picture").unwrap();
        let outcome = transfer(
            &local.path().join("bild.jpg"),
            &folder.join("fotos").join("bild.jpg"),
            CopyMode::Copy,
            false,
        );
        assert!(
            matches!(outcome, LocalOutcome::Completed { .. }),
            "{outcome:?}"
        );
        assert_eq!(names(&folder), ["Fotos"], "{}", volume.display());
        assert_eq!(names(&folder.join("Fotos")), ["bild.jpg"]);
        assert_eq!(
            std::fs::read(folder.join("Fotos/bild.jpg")).unwrap(),
            b"picture"
        );
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
