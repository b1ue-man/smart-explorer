//! E11 (plan 2026-11-07), Share file operations: what an unlocked vault drive
//! shows under an export is no file of the export, a locked vault container
//! stays an ordinary encrypted folder that another machine opens and unlocks
//! itself.
use super::*;
use crate::share::{
    fs_host_policy::in_vault_drive,
    fs_host_policy_task_tests::{text, vault_fixture, Published},
    profile_edits::refuse_vault_exports,
    ShareProfiles,
};
use crate::vfs::Backend;
use std::{io::Read, io::Write, path::PathBuf};

fn exports(roots: &[(&str, PathBuf)]) -> Arc<Mutex<ShareExportConfig>> {
    Arc::new(Mutex::new(ShareExportConfig {
        roots: roots
            .iter()
            .map(|(label, path)| {
                SharedRoot::new(*label, text(path)).with_access(ExportAccess::ReadWrite)
            })
            .collect(),
        ..Default::default()
    }))
}

fn names(path: &str, exports: &Arc<Mutex<ShareExportConfig>>) -> Vec<String> {
    list_dir(path, exports)
        .unwrap()
        .into_iter()
        .map(|entry| entry.name)
        .collect()
}

fn refusal<T>(result: io::Result<T>) -> io::ErrorKind {
    result.err().expect("the operation is refused").kind()
}

/// M16-c: under an export that holds the drive, the listing leaves it out
/// and every operation on it is a permission refusal; an export whose
/// folder is the drive itself resolves to nothing. The drive is ordinary
/// again once it ended. E35: `in_vault_drive` (spelling, then identity)
/// names the same drive, a file below it and the drive root alike, while the
/// drive is mounted; this test also absorbs the function-level check of the
/// first run (`fs_tresor_task_reported_mount_is_private_for_share_host`).
#[test]
fn vault_task_share_host_hides_and_refuses_an_unlocked_vault_drive() {
    let (_directory, folder, drive) = vault_fixture();
    let drive_text = drive.to_string_lossy().into_owned();
    assert!(
        !in_vault_drive(&drive_text),
        "private before the drive reports"
    );
    let published = Published::new(&drive);
    assert!(in_vault_drive(&drive_text), "the drive root is private");
    assert!(
        in_vault_drive(&drive.join("geheim.txt").to_string_lossy()),
        "a file below the drive is private"
    );
    let docs = exports(&[("Docs", folder.clone())]);
    let listed = names("/Docs", &docs);
    assert!(listed.contains(&"frei.txt".to_string()), "{listed:?}");
    assert!(!listed.contains(&"T".to_string()), "{listed:?}");
    let denied = io::ErrorKind::PermissionDenied;
    assert_eq!(refusal(stat("/Docs/T/geheim.txt", &docs)), denied);
    assert_eq!(refusal(resolve("/Docs/T/geheim.txt", &docs)), denied);
    assert_eq!(refusal(resolve("/Docs/T/sub", &docs)), denied);
    assert_eq!(refusal(list_dir("/Docs/T", &docs)), denied);
    stat("/Docs/frei.txt", &docs).unwrap();
    resolve("/Docs/frei.txt", &docs).unwrap();

    let vault = exports(&[("Tresor", drive.clone())]);
    assert_eq!(refusal(resolve("/Tresor/geheim.txt", &vault)), denied);
    assert_eq!(refusal(resolve("/Tresor", &vault)), denied);
    assert_eq!(refusal(list_dir("/Tresor", &vault)), denied);

    drop(published);
    assert!(
        !in_vault_drive(&drive_text),
        "shareable again after the withdrawal"
    );
    assert!(names("/Docs", &docs).contains(&"T".to_string()));
    assert_eq!(stat("/Docs/T/geheim.txt", &docs).unwrap().size, 8);
    resolve("/Tresor/geheim.txt", &vault).unwrap();
}

/// M16-f: the encrypted container of a vault is files like any others, also
/// while an unlocked drive of another vault is published beside it: list,
/// read, write and rename work, and the folder may be exported.
#[test]
fn vault_task_share_host_serves_a_locked_vault_container_like_any_folder() {
    let (_directory, folder, drive) = vault_fixture();
    let container = folder.join("Container");
    let leaf = "d/AB/CDEF0123456789ABCDEF0123456789";
    std::fs::create_dir_all(container.join(leaf)).unwrap();
    std::fs::write(container.join("vault.cryptomator"), b"jwt").unwrap();
    std::fs::write(container.join("masterkey.cryptomator"), b"{}").unwrap();
    std::fs::write(container.join(leaf).join("x.c9r"), b"cipher").unwrap();
    let _published = Published::new(&drive);
    let docs = exports(&[("Docs", folder.clone())]);

    let top = names("/Docs/Container", &docs);
    for wanted in ["vault.cryptomator", "masterkey.cryptomator", "d"] {
        assert!(top.contains(&wanted.to_string()), "{wanted} in {top:?}");
    }
    let inner = format!("/Docs/Container/{leaf}");
    assert_eq!(names(&inner, &docs), vec!["x.c9r".to_string()]);

    let mut content = Vec::new();
    let source = resolve(&format!("{inner}/x.c9r"), &docs).unwrap();
    source
        .backend
        .open_read(&source.path)
        .unwrap()
        .read_to_end(&mut content)
        .unwrap();
    assert_eq!(content, b"cipher");

    let created = resolve(&format!("{inner}/y.c9r"), &docs).unwrap();
    let mut writer = created.backend.open_write(&created.path).unwrap();
    writer.write_all(b"new cipher").unwrap();
    drop(writer);
    rename(
        &format!("{inner}/y.c9r"),
        &format!("{inner}/z.c9r"),
        &docs,
        false,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(container.join(leaf).join("z.c9r")).unwrap(),
        b"new cipher"
    );
    assert!(!container.join(leaf).join("y.c9r").exists());

    // The container may be an export; the drive beside it may not.
    let mut profiles = ShareProfiles::default();
    profiles
        .default_direct_exports
        .roots
        .push(SharedRoot::new("Container", text(&container)));
    refuse_vault_exports(&[], &profiles).unwrap();
    profiles
        .default_direct_exports
        .roots
        .push(SharedRoot::new("Tresor", text(&drive)));
    assert!(refuse_vault_exports(&[], &profiles).is_err());
}
