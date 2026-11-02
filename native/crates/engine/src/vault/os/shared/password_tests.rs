//! Password change (VLT-M5, stage `vault_unit`, C3.7): never without a
//! valid masterkey, never a moment without a valid key file.
use std::io::{self, Read, Write};
use std::sync::Arc;

use crate::cryptomator::{backup_file_name, FormatError, MASTERKEY_FILE};
use crate::vfs::{Backend, LocalBackend, Scheme, VfsMeta, VfsResult};

use super::backend::VaultBackend;
use super::backend_test_support::{fwd, read_with, write_with};
use super::manage::create_vault;
use super::password::{change_password, PasswordError};
use super::unlock::{session_for, unlock_storage, UnlockError};

const OLD: &str = "altes-Passwort-1";
const NEW: &str = "neues-Passwort-2";

/// The local storage without an atomic replace (as some FTP servers).
struct NoReplace(LocalBackend);

impl Backend for NoReplace {
    fn scheme(&self) -> Scheme {
        Scheme::Local
    }
    fn root_display(&self) -> String {
        self.0.root_display()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.0.list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.0.stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.0.open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.0.open_write(path)
    }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.0.open_write_new(path)
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.0.rename(src, dst)
    }
    fn promote_staged(&self, _staged: &str, _destination: &str) -> VfsResult<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "kein atomares Ersetzen",
        ))
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.0.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.0.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.0.mkdir_all(path)
    }
}

fn names(root: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .expect("vault root")
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

fn key_file(root: &str) -> Vec<u8> {
    std::fs::read(format!("{root}/{MASTERKEY_FILE}")).expect("key file")
}

#[test]
fn vault_task_password_change_never_without_masterkey() {
    let dir = tempfile::tempdir().expect("temporary folder");
    let local = LocalBackend::new("/");
    let root = format!("{}/Tresor", fwd(dir.path()));
    let other = format!("{}/Fremd", fwd(dir.path()));
    create_vault(&local, &root, OLD, 220, None).expect("vault");
    create_vault(&local, &other, OLD, 220, None).expect("second vault");
    let session = session_for("0123456789abcdef0123456789abcdef", &local, &root, OLD).unwrap();
    let vault = VaultBackend::new(
        Arc::new(LocalBackend::new("/")),
        &root,
        Arc::clone(&session),
    );
    write_with(&vault, "/inhalt.txt", b"bleibt lesbar");
    let original = key_file(&root);
    let before = names(&root);
    let (old_key, _) = unlock_storage(&local, &root, OLD, None).unwrap();

    // A wrong old password ends before any write.
    let wrong = change_password(&local, &root, None, "falsches-Passwort", NEW, None);
    assert_eq!(
        wrong.unwrap_err(),
        PasswordError::Unlock(UnlockError::Format(FormatError::WrongPassword))
    );
    assert_eq!(
        (key_file(&root), names(&root)),
        (original.clone(), before.clone())
    );
    // A key file that unwraps but is not this vault's masterkey (the key
    // file of another vault with the same password) is refused unwritten.
    std::fs::write(format!("{root}/{MASTERKEY_FILE}"), key_file(&other)).unwrap();
    let foreign = change_password(&local, &root, None, OLD, NEW, None);
    assert_eq!(
        foreign.unwrap_err(),
        PasswordError::Unlock(UnlockError::Format(FormatError::ConfigSignature))
    );
    assert_eq!(key_file(&root), key_file(&other));
    assert_eq!(names(&root), before);
    std::fs::write(format!("{root}/{MASTERKEY_FILE}"), &original).unwrap();
    // Another vault ID than the entry's.
    let moved = change_password(&local, &root, Some("andere-jti"), OLD, NEW, None);
    assert_eq!(
        moved.unwrap_err(),
        PasswordError::Unlock(UnlockError::OtherVault)
    );
    // A storage without an atomic replace keeps the old key file; no stage
    // and no backup stay behind.
    let refused = change_password(
        &NoReplace(LocalBackend::new("/")),
        &root,
        None,
        OLD,
        NEW,
        None,
    );
    assert_eq!(refused.unwrap_err(), PasswordError::NotAtomic);
    assert_eq!(PasswordError::NotAtomic.code(), Some("vault_not_atomic"));
    assert_eq!(
        (key_file(&root), names(&root)),
        (original.clone(), before.clone())
    );

    // The change: the old file kept as `.bkup`, the new one in place in one
    // step; the masterkey and every file stay.
    let changed = change_password(&local, &root, None, OLD, NEW, None).expect("changed");
    assert_eq!(changed.backup, backup_file_name(MASTERKEY_FILE, &original));
    assert!(changed.backup.ends_with(".bkup"));
    assert_eq!(
        std::fs::read(format!("{root}/{}", changed.backup)).unwrap(),
        original
    );
    let mut after = before.clone();
    after.push(changed.backup.clone());
    after.sort();
    assert_eq!(names(&root), after, "no stage left");
    let (new_key, _) = unlock_storage(&local, &root, NEW, None).expect("new password");
    assert_eq!(new_key.raw(), old_key.raw(), "same masterkey");
    assert_eq!(
        unlock_storage(&local, &root, OLD, None).unwrap_err(),
        UnlockError::Format(FormatError::WrongPassword)
    );
    let reopened = session_for("0123456789abcdef0123456789abcdef", &local, &root, NEW).unwrap();
    let vault = VaultBackend::new(Arc::new(LocalBackend::new("/")), &root, reopened);
    assert_eq!(read_with(&vault, "/inhalt.txt"), b"bleibt lesbar");
    // An open session goes on: the masterkey did not change.
    assert!(!session.is_locked());
}
