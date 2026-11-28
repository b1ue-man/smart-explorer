use super::*;
use std::path::PathBuf;

const GUID: &str = "Volume{0b1c2d3e-4f50-6172-8394-a5b6c7d8e9f0}";

/// A mount-point reparse buffer whose substitute name is `target`.
fn buffer(tag: u32, target: &str) -> Vec<u8> {
    let substitute: Vec<u8> = target.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let print: Vec<u8> = "print".encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut data = Vec::new();
    data.extend(tag.to_le_bytes());
    data.extend(((8 + substitute.len() + 2 + print.len() + 2) as u16).to_le_bytes());
    data.extend(0u16.to_le_bytes());
    data.extend(0u16.to_le_bytes()); // substitute offset
    data.extend((substitute.len() as u16).to_le_bytes());
    data.extend(((substitute.len() + 2) as u16).to_le_bytes()); // print offset
    data.extend((print.len() as u16).to_le_bytes());
    data.extend(&substitute);
    data.extend([0, 0]);
    data.extend(&print);
    data.extend([0, 0]);
    data
}

#[test]
fn removable_task_mount_point_onto_a_volume_is_no_link() {
    let root = format!(r"\\?\{GUID}\");
    assert_eq!(
        mounted_volume_root(&buffer(MOUNT_POINT_TAG, &format!(r"\??\{GUID}\"))),
        Some(root.clone())
    );
    assert_eq!(
        mounted_volume_root(&buffer(MOUNT_POINT_TAG, &format!(r"\??\{GUID}"))),
        Some(root)
    );
    // Junctions (a folder on a drive, a folder below a volume) stay links,
    // as does every other reparse tag and a truncated buffer.
    for target in [
        r"\??\C:\target".to_string(),
        format!(r"\??\{GUID}\folder"),
        r"\??\Volume{0b1c2d3e}\".to_string(),
        format!(r"\\?\{GUID}\"),
    ] {
        assert_eq!(
            mounted_volume_root(&buffer(MOUNT_POINT_TAG, &target)),
            None,
            "{target}"
        );
    }
    let symlink = buffer(0xA000_000C, &format!(r"\??\{GUID}\"));
    assert_eq!(mounted_volume_root(&symlink), None);
    let whole = buffer(MOUNT_POINT_TAG, &format!(r"\??\{GUID}\"));
    for cut in [0, 3, 12, 20, whole.len() / 2] {
        assert_eq!(mounted_volume_root(&whole[..cut]), None, "cut at {cut}");
    }
}

fn run(program: &str, arguments: &[&str]) -> String {
    let output = std::process::Command::new(program)
        .args(arguments)
        .output()
        .unwrap_or_else(|error| panic!("{program}: {error}"));
    assert!(
        output.status.success(),
        "{program} {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
#[ignore = "needs a FAT32 (SE_TEST_FAT32_DIR) and exFAT (SE_TEST_EXFAT_DIR) directory on Windows; Suite crates-task"]
fn removable_task_folder_a_volume_is_mounted_in_lists_that_volume() {
    // The FAT32 volume of the suite (a drive-letter path, assumed) is also
    // mounted in an empty NTFS folder; the folder must list the volume.
    let fat = PathBuf::from(std::env::var_os("SE_TEST_FAT32_DIR").expect("SE_TEST_FAT32_DIR"));
    let drive = fat.to_string_lossy()[..3].to_string();
    let guid = run("mountvol", &[drive.as_str(), "/L"]).trim().to_string();
    assert!(guid.starts_with(r"\\?\Volume{"), "{guid}");
    let marker = format!("se-mount-marker-{}", std::process::id());
    std::fs::write(PathBuf::from(&drive).join(&marker), b"on the volume").unwrap();
    let parent = std::env::temp_dir().join(format!("se-mount-{}", std::process::id()));
    let folder = parent.join("usb");
    std::fs::create_dir_all(&folder).unwrap();
    let mount = folder.to_string_lossy().into_owned();
    run("mountvol", &[mount.as_str(), guid.as_str()]);
    let listed = (|| -> io::Result<(Vec<String>, bool, bool, [bool; 2])> {
        let names = crate::local_access::read_directory(&folder)?
            .map(|entry| entry.map(|entry| entry.name.to_string_lossy().into_owned()))
            .collect::<io::Result<Vec<_>>>()?;
        let pinned = super::super::DirectoryHandle::open_root(&folder)?;
        let pinned_lists = pinned
            .read_directory()?
            .filter_map(Result::ok)
            .any(|entry| entry.name.to_string_lossy() == marker);
        // In its parent's listing the folder stays a boundary: a walk of the
        // parent does not cross onto the mounted volume.
        let boundary = crate::local_access::read_directory(&parent)?
            .filter_map(Result::ok)
            .any(|entry| entry.name == "usb" && entry.is_link_like);
        let classified = [
            is_volume_mount_point(&folder),
            is_volume_mount_point(&parent),
        ];
        Ok((names, pinned_lists, boundary, classified))
    })();
    run("mountvol", &[mount.as_str(), "/D"]);
    let _ = std::fs::remove_file(PathBuf::from(&drive).join(&marker));
    let _ = std::fs::remove_dir_all(&parent);
    let (names, pinned_lists, boundary, classified) = listed.unwrap();
    assert_eq!(classified, [true, false], "mount folder, parent");
    assert!(names.contains(&marker), "{names:?}");
    assert!(pinned_lists, "the pinned root is the mounted volume");
    assert!(
        boundary,
        "the mount folder stays a walk boundary in its parent"
    );
}
