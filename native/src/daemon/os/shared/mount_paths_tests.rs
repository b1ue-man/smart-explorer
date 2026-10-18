//! Signal of `mount_path_for` (DRV-M7, stage drive-vault; S2-V8): a vault
//! location maps to the folder (Linux) or letter (Windows) of the mounted
//! drive of its site, names the drive cannot show and `..` map to nothing,
//! and Google Drive locations map below the drive root.
use std::path::{Path, PathBuf};

use super::{drive_path, mounted_root, path_on_drive, relative_in, Located};
use crate::mount::{
    BackendRoot, DriveLetter, DriveSelection, MountConfig, MountId, MountMode, MountRecovery,
    MountSnapshot, MountSource, MountStatus, PathDialect,
};

const SITE: &str = "0123456789abcdef0123456789abcdef";

fn snapshot(id: &str, source: MountSource, status: MountStatus) -> MountSnapshot {
    let config = MountConfig::new(
        MountId::parse(id).unwrap(),
        source,
        DriveSelection::Automatic,
        MountMode::ReadWrite,
        "Laufwerk",
    )
    .unwrap();
    MountSnapshot {
        config,
        status,
        recovery: MountRecovery::Clean,
        recovery_required_compat: false,
    }
}

fn vault_at(location: &str) -> Located {
    Located::parse(&format!("vault://{SITE}{location}")).unwrap()
}

#[test]
fn drive_task_mount_path_for_maps_drive_paths() {
    let vault = MountSource::Vault {
        site_id: SITE.into(),
    };
    let folder = PathBuf::from("/run/user/1000/smart-explorer/Tresore/Privat");
    let linux = snapshot(
        "vault-linux",
        vault.clone(),
        MountStatus::MountedAt {
            path: folder.clone(),
        },
    );
    let drive = DriveLetter::parse('V').unwrap();
    let windows = snapshot(
        "vault-windows",
        vault.clone(),
        MountStatus::Mounted { drive },
    );

    let picture = vault_at("/Bilder/Urlaub ä.jpg");
    assert_eq!(
        picture,
        Located::Vault {
            site: SITE.into(),
            path: "/Bilder/Urlaub ä.jpg".into(),
        }
    );
    assert_eq!(
        drive_path(&linux, &picture, SITE),
        Some(folder.join("Bilder").join("Urlaub ä.jpg"))
    );
    let mut on_letter = PathBuf::from("V:\\");
    on_letter.push("Bilder");
    on_letter.push("Urlaub ä.jpg");
    assert_eq!(drive_path(&windows, &picture, SITE), Some(on_letter));
    // The vault root is the drive root.
    let root = Located::parse(&format!("vault://{SITE}")).unwrap();
    assert_eq!(drive_path(&linux, &root, SITE), Some(folder.clone()));
    // A name Windows cannot show exists only on the Linux drive.
    let colon = vault_at("/a:b.txt");
    assert_eq!(drive_path(&windows, &colon, SITE), None);
    assert_eq!(
        drive_path(&linux, &colon, SITE),
        Some(folder.join("a:b.txt"))
    );
    // `..` never leaves the drive.
    assert_eq!(drive_path(&linux, &vault_at("/../x"), SITE), None);
    // Another site and a drive that waits for the unlock serve nothing.
    let other_site = "ffffffffffffffffffffffffffffffff";
    assert_eq!(drive_path(&linux, &picture, other_site), None);
    let waiting = snapshot(
        "vault-wait",
        vault,
        MountStatus::Failed {
            detail: "vault_locked: Tresor gesperrt".into(),
        },
    );
    assert_eq!(drive_path(&waiting, &picture, SITE), None);

    // Google Drive maps below the drive's root.
    let gdrive = snapshot(
        "gdrive-team",
        MountSource::GoogleDrive {
            account: "cloud:gdrive".into(),
            root: BackendRoot::parse("/Team").unwrap(),
        },
        MountStatus::MountedAt {
            path: PathBuf::from("/home/u/SmartExplorer/Team"),
        },
    );
    let plan = Located::parse("gdrive:///Team/Plan.txt").unwrap();
    assert_eq!(
        drive_path(&gdrive, &plan, ""),
        Some(PathBuf::from("/home/u/SmartExplorer/Team/Plan.txt"))
    );
    let beside = Located::parse("gdrive:///Teamwork/x").unwrap();
    assert_eq!(drive_path(&gdrive, &beside, ""), None);
    let team = Located::parse("gdrive:///Team").unwrap();
    assert_eq!(
        drive_path(&gdrive, &team, ""),
        Some(PathBuf::from("/home/u/SmartExplorer/Team"))
    );

    // Locations no drive kind serves.
    assert_eq!(Located::parse("/home/u/a.txt"), None);
    assert_eq!(Located::parse("sftp://user@host/a"), None);
    // The parts on their own.
    let source = MountSource::Vault {
        site_id: SITE.into(),
    };
    assert_eq!(
        relative_in(&source, &picture, SITE).as_deref(),
        Some("Bilder/Urlaub ä.jpg")
    );
    assert!(mounted_root(&MountStatus::Mounting).is_none());
    assert_eq!(
        path_on_drive(Path::new("/m"), PathDialect::Posix, ""),
        Some(PathBuf::from("/m"))
    );
}
