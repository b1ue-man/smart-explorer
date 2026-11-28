//! Name rules of filesystem profiles: which spellings are one name and the
//! German reasons a file is not copied (U7, U11).
use super::*;
use crate::types::Win32NameIssue;

#[test]
fn wechselmedium_task_fat_and_exfat_fold_names_ext4_and_ntfs3_do_not() {
    let vfat = linux_profile("vfat").names;
    assert_eq!(vfat.folding, CaseFolding::Ascii);
    assert_eq!(vfat.folding.key("a.TXT"), vfat.folding.key("A.txt"));
    assert_ne!(
        vfat.folding.key("ä.txt"),
        vfat.folding.key("Ä.txt"),
        "Linux vfat (utf8) folds ASCII only"
    );
    let exfat = linux_profile("exfat").names;
    assert_eq!(exfat.folding.key("ä.txt"), exfat.folding.key("Ä.txt"));
    for kept in ["ext4", "ntfs3", "fuseblk", "cifs"] {
        assert_eq!(linux_profile(kept).names.folding.key("A"), None, "{kept}");
    }
    let windows = windows_profile("NTFS", 255).names;
    assert_eq!(windows.folding, CaseFolding::Unicode);
    assert_eq!(windows.label, Some("NTFS"));
    assert_eq!(
        windows.folding.key("straße"),
        Some("STRAßE".to_string()),
        "an up-case table maps letter to letter"
    );
}

#[test]
fn wechselmedium_task_reasons_name_the_filesystem_and_the_limit() {
    let fat = linux_profile("vfat");
    let limit = fat.limits.max_file_size.expect("FAT has a file size limit");
    assert_eq!(
        fat.names.size_reason(limit),
        "größer als 4 GiB − 1 Byte: FAT kann sie nicht speichern"
    );
    assert_eq!(
        windows_profile("FAT32", 255).names.size_reason(limit),
        "größer als 4 GiB − 1 Byte: FAT32 kann sie nicht speichern"
    );
    assert_eq!(
        fat.names.name_reason(
            NameIssue::Windows(Win32NameIssue::InvalidCharacter),
            fat.limits.max_name
        ),
        "Name nicht erlaubt auf FAT: enthält unter Windows ungültige Zeichen"
    );
    assert_eq!(
        linux_profile("ext4")
            .names
            .name_reason(NameIssue::TooLong, Some(NameLimit::Bytes(255))),
        "Name nicht erlaubt auf dem Ziel-Dateisystem: länger als 255 Byte"
    );
    let fold = fat.names.fold_reason("a.txt", "A.txt");
    assert!(
        fold.contains("„a.txt“") && fold.contains("„A.txt“"),
        "{fold}"
    );
}
