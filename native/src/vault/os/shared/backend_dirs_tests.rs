//! Folders of the vault: creation order and removal (VLT-M2, stage
//! `vault_unit`).
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

use crate::cryptomator::{decrypt_bytes, CipherCombo};
use crate::vfs::{Backend, BackendHandle};

use super::backend_test_support::{Recording, TestVault};

fn err<T>(result: io::Result<T>) -> io::Error {
    match result {
        Ok(_) => panic!("an error was expected"),
        Err(error) => error,
    }
}

#[test]
fn vault_task_backend_mkdir_order_content_before_node() {
    let recording = Recording::new();
    let storage = Arc::clone(&recording);
    let vault = TestVault::on(CipherCombo::SivGcm, 220, move |root| {
        storage.set_root(root);
        let handle: BackendHandle = storage;
        handle
    });
    let backend = &vault.backend;
    backend.create_dir("/Docs").unwrap();
    let log = recording.entries();
    let docs = vault.content("/Docs");
    let node = format!("{}/{}", vault.content("/"), vault.encrypted("/", "Docs"));
    let position = |entry: String| {
        log.iter()
            .position(|logged| *logged == entry)
            .unwrap_or_else(|| panic!("{entry} missing in {log:?}"))
    };
    let content_created = position(format!("mkdir {docs}"));
    let backup_written = position(format!("write {docs}/dirid.c9r"));
    let node_created = position(format!("mkdir {node}"));
    let id_written = position(format!("write {node}/dir.c9r"));
    assert!(
        content_created < backup_written
            && backup_written < node_created
            && node_created < id_written,
        "{log:?}"
    );
    // `dirid.c9r` holds the folder's own ID, encrypted; `dir.c9r` the plain ID.
    let id = vault.dir_id("/Docs");
    let backup = std::fs::read(vault.at(&format!("{docs}/dirid.c9r"))).unwrap();
    let decrypted = decrypt_bytes(&vault.session.cryptor(), &backup, 64).unwrap();
    assert_eq!(&decrypted[..], id.as_bytes());
    let plain = std::fs::read(vault.at(&format!("{node}/dir.c9r"))).unwrap();
    assert_eq!(plain, id.as_bytes());
    // An existing folder: `create_dir` succeeds, `create_dir_new` refuses.
    backend.create_dir("/Docs").unwrap();
    assert_eq!(
        err(backend.create_dir_new("/Docs")).kind(),
        io::ErrorKind::AlreadyExists
    );
    backend.mkdir_all("/A/B/C").unwrap();
    assert!(backend.stat("/A/B/C").unwrap().is_dir);
    assert_eq!(
        backend
            .list_dir("/A/B")
            .unwrap()
            .iter()
            .map(|meta| meta.name.as_str())
            .collect::<Vec<_>>(),
        ["C"]
    );
}

#[test]
fn vault_task_backend_rmdir_nonempty_refused() {
    let vault = TestVault::new(CipherCombo::SivGcm, 220);
    let backend = &vault.backend;
    backend.create_dir("/Docs").unwrap();
    vault.write("/Docs/a.txt", b"a");
    let docs = vault.content("/Docs");
    let node = format!("{}/{}", vault.content("/"), vault.encrypted("/", "Docs"));
    assert_eq!(
        err(backend.remove_dir("/Docs")).kind(),
        io::ErrorKind::DirectoryNotEmpty
    );
    assert_eq!(vault.read("/Docs/a.txt"), b"a");
    assert!(backend.stat("/Docs").unwrap().is_dir);
    // A stage nobody sees in the listing keeps the folder as well.
    let stage = "/Docs/b.txt.se-copy-0123456789abcdef";
    let mut writer = backend.open_write_copy_stage(stage).unwrap();
    writer.write_all(b"b").unwrap();
    writer.flush().unwrap();
    drop(writer);
    backend.remove_file("/Docs/a.txt").unwrap();
    assert!(backend.list_dir("/Docs").unwrap().is_empty());
    assert_eq!(
        err(backend.remove_dir("/Docs")).kind(),
        io::ErrorKind::DirectoryNotEmpty
    );
    backend.discard_copy_stage(stage).unwrap();
    // Empty: node and content directory go.
    backend.remove_dir("/Docs").unwrap();
    assert!(!Path::new(&vault.at(&node)).exists());
    assert!(!Path::new(&vault.at(&docs)).exists());
    assert_eq!(err(backend.stat("/Docs")).kind(), io::ErrorKind::NotFound);
    // The root and files are no folders to remove.
    assert_eq!(
        err(backend.remove_dir("/")).kind(),
        io::ErrorKind::InvalidInput
    );
    vault.write("/f.txt", b"f");
    assert_eq!(
        err(backend.remove_dir("/f.txt")).kind(),
        io::ErrorKind::NotADirectory
    );
}
