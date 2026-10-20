//! Folder rules of Linux drives (DRV-M4, decision C2): names and default
//! places, preparation with lock and open modes, refusals, the vault place
//! below `XDG_RUNTIME_DIR`, and the `mountinfo` fields `--force` relies on.
use super::super::fuse_abort::{abort_in, mount_in, parse_mountinfo};
use super::{
    default_location, folder_is_free, folder_name, is_tmpfs_magic, lock, open_for_mount, prepare,
    vaults_root_in, PlaceKind,
};
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

#[test]
fn mount_point_folder_names_and_default_places() {
    assert_eq!(folder_name("Smart Explorer"), "Smart Explorer");
    assert_eq!(folder_name(" prod/srv "), "prod_srv");
    assert_eq!(folder_name("..hidden"), "hidden");
    assert_eq!(folder_name("a\nb"), "a_b");
    assert_eq!(folder_name(""), "Laufwerk");
    assert_eq!(folder_name("..."), "Laufwerk");
    let long = "ä".repeat(200);
    assert!(folder_name(&long).len() <= 255);
    let root = Path::new("/home/u/SmartExplorer");
    let first = default_location(root, "prod", |_| false).unwrap();
    assert_eq!(first, root.join("prod"));
    let taken = [root.join("prod"), root.join("prod (2)")];
    let third = default_location(root, "prod", |path| taken.iter().any(|t| t == path)).unwrap();
    assert_eq!(third, root.join("prod (3)"));
    assert!(vaults_root_in(None).is_err());
    assert!(vaults_root_in(Some(PathBuf::from("relative"))).is_err());
    assert_eq!(
        vaults_root_in(Some(PathBuf::from("/run/user/1000"))).unwrap(),
        Path::new("/run/user/1000/smart-explorer/Tresore")
    );
    assert!(is_tmpfs_magic(0x0102_1994_i64));
    assert!(is_tmpfs_magic(0x0102_1994_u64));
    assert!(!is_tmpfs_magic(0xEF53_i64));
}

#[test]
fn mount_point_prepare_locks_opens_and_refuses() {
    let base = tempfile::tempdir().unwrap();
    let folder = base.path().join("drives").join("prod");
    assert!(folder_is_free(&folder));
    prepare(&folder, PlaceKind::Drive).unwrap();
    assert!(folder.is_dir());
    assert_eq!(mode(&folder), 0o500);
    assert!(fs::write(folder.join("x"), b"x").is_err() || unsafe { libc::geteuid() } == 0);
    open_for_mount(&folder, PlaceKind::Drive).unwrap();
    assert_eq!(mode(&folder), 0o700);
    lock(&folder, PlaceKind::Drive).unwrap();
    assert_eq!(mode(&folder), 0o500);
    // Preparing again keeps the empty folder usable.
    prepare(&folder, PlaceKind::Drive).unwrap();
    assert!(folder_is_free(&folder));

    let full = base.path().join("full");
    fs::create_dir(&full).unwrap();
    fs::write(full.join("kept.txt"), b"kept").unwrap();
    assert!(!folder_is_free(&full));
    let error = prepare(&full, PlaceKind::Drive).unwrap_err();
    assert!(error.to_string().contains("ist nicht leer"), "{error}");
    assert_eq!(fs::read(full.join("kept.txt")).unwrap(), b"kept");

    let link = base.path().join("link");
    symlink(&full, &link).unwrap();
    let error = prepare(&link, PlaceKind::Drive).unwrap_err();
    assert!(error.to_string().contains("ist kein Ordner"), "{error}");

    let file = base.path().join("file");
    fs::write(&file, b"").unwrap();
    assert!(prepare(&file, PlaceKind::Drive).is_err());
    assert!(prepare(Path::new("relative/drive"), PlaceKind::Drive).is_err());

    // A vault folder keeps the sticky bit; outside tmpfs it is refused.
    let shm = Path::new("/dev/shm");
    if shm.is_dir() {
        let vault_base = tempfile::tempdir_in(shm).unwrap();
        let vault = vault_base.path().join("Tresor");
        prepare(&vault, PlaceKind::Vault).unwrap();
        assert_eq!(mode(&vault), 0o1500);
        open_for_mount(&vault, PlaceKind::Vault).unwrap();
        assert_eq!(mode(&vault), 0o1700);
        lock(&vault, PlaceKind::Vault).unwrap();
        assert_eq!(mode(&vault), 0o1500);
    }
}

#[test]
fn fuse_abort_reads_mountinfo_fields_and_escapes() {
    let text = "\
22 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw
95 22 0:52 / /home/u/SmartExplorer/my\\040drive rw,nosuid,nodev,noatime shared:50 - fuse.smartexplorer smartexplorer rw,user_id=1000,group_id=1000
96 95 0:53 / /home/u/SmartExplorer/my\\040drive rw - fuse.smartexplorer smartexplorer rw
97 22 0:54 / /tmp/back\\134slash\\011tab rw master:2 - tmpfs tmpfs rw
broken line without separator
";
    let entries = parse_mountinfo(text);
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[1].fstype, "fuse.smartexplorer");
    assert_eq!(entries[1].major, 0);
    assert_eq!(entries[1].minor, 52);
    assert_eq!(entries[3].mount_point, Path::new("/tmp/back\\slash\ttab"));
    // A stacked mount: the top-most (last) entry counts.
    let top = mount_in(text, Path::new("/home/u/SmartExplorer/my drive")).unwrap();
    assert_eq!(top.minor, 53);
    assert!(mount_in(text, Path::new("/home/u/SmartExplorer")).is_none());

    // The abort file of a connection; without fusectl the error says so.
    let connections = tempfile::tempdir().unwrap();
    let error = abort_in(connections.path(), 53).unwrap_err();
    assert!(error.to_string().contains("fusectl"), "{error}");
    fs::create_dir(connections.path().join("53")).unwrap();
    fs::write(connections.path().join("53").join("abort"), b"").unwrap();
    abort_in(connections.path(), 53).unwrap();
    assert_eq!(
        fs::read(connections.path().join("53").join("abort")).unwrap(),
        b"1"
    );
}

/// R15c: past the former 999 suffixes the search goes on to a free name;
/// only a real error (a root that is a file) ends it.
#[test]
fn drive_task_mount_point_name_search_has_no_limit() {
    let root = Path::new("/home/u/SmartExplorer");
    let taken = |path: &Path| {
        let name = path.file_name().unwrap().to_string_lossy();
        name == "prod"
            || name
                .strip_prefix("prod (")
                .and_then(|rest| rest.strip_suffix(')'))
                .and_then(|number| number.parse::<u32>().ok())
                .is_some_and(|number| number <= 1500)
    };
    assert_eq!(
        default_location(root, "prod", taken).unwrap(),
        root.join("prod (1501)")
    );
    let base = tempfile::tempdir().unwrap();
    let file = base.path().join("not-a-folder");
    fs::write(&file, b"x").unwrap();
    let error = default_location(&file, "prod", |_| true).unwrap_err();
    assert_ne!(error.kind(), std::io::ErrorKind::NotFound);
}
