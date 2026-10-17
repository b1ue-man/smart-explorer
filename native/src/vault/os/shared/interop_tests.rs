//! Interop with Cryptomator's own library (VLT-M5, stage `vault-interop`,
//! fixture `interop`): `SE_VAULT_INTEROP_JAVA -jar SE_VAULT_INTEROP_JAR
//! <command> --vault DIR --password-file FILE …` runs CryptoFS 2.11.0
//! (`native/vault-task-interop/VaultTaskInterop.java`; exit 0 ok, 1
//! difference or finding, 2 does not unlock, 3 usage or I/O).
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::cryptomator::CipherCombo;
use crate::vfs::{Backend, LocalBackend};

use super::backend::VaultBackend;
use super::backend_test_support::{fwd, pattern, read_with, write_with};
use super::manage::create_vault;
use super::password::change_password;
use super::unlock::session_for;

const SITE: &str = "0123456789abcdef0123456789abcdef";
const PASSWORD: &str = "Prüf-Passwort-2026";
const NEW: &str = "Neues-Prüf-Passwort-27";

struct Harness {
    java: PathBuf,
    jar: PathBuf,
}

impl Harness {
    fn from_env() -> Self {
        let var = |name: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    panic!("{name} fehlt: nur in der Stufe vault-interop (fixture interop)")
                })
        };
        Self {
            java: var("SE_VAULT_INTEROP_JAVA"),
            jar: var("SE_VAULT_INTEROP_JAR"),
        }
    }

    /// Exit code and output of one harness command.
    fn run(&self, args: &[&str]) -> (i32, String) {
        let output = Command::new(&self.java)
            .arg("-jar")
            .arg(&self.jar)
            .args(args)
            .output()
            .expect("Java des Interop-Harness startet");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        (output.status.code().unwrap_or(-1), text)
    }

    fn expect(&self, code: i32, args: &[&str]) {
        let (got, output) = self.run(args);
        assert_eq!(got, code, "{args:?}: {output}");
    }
}

/// The files of the tests: long name (shortened), umlauts, empty file,
/// exactly one chunk, three chunks, deep folders.
fn files() -> Vec<(String, Vec<u8>)> {
    vec![
        ("/a.txt".into(), b"hallo tresor\n".to_vec()),
        ("/Ordner/ü-ä.txt".into(), "Umlaute".as_bytes().to_vec()),
        ("/leer.txt".into(), Vec::new()),
        (format!("/lang-{}.bin", "x".repeat(195)), pattern(70_000)),
        ("/genau-ein-block.bin".into(), pattern(32 * 1024)),
        ("/tief/a/b/c.txt".into(), b"tief".to_vec()),
    ]
}

/// `{"files": {"/path": "<sha256 hex>"}}` as the harness reads it.
fn manifest(dir: &std::path::Path, files: &[(String, Vec<u8>)]) -> String {
    let entries: BTreeMap<&str, String> = files
        .iter()
        .map(|(path, bytes)| {
            let digest = Sha256::digest(bytes);
            (path.as_str(), data_encoding::HEXLOWER.encode(&digest))
        })
        .collect();
    let path = format!("{}/manifest.json", fwd(dir));
    std::fs::write(&path, serde_json::json!({ "files": entries }).to_string()).unwrap();
    path
}

fn password_file(dir: &std::path::Path, name: &str, password: &str) -> String {
    let path = format!("{}/{name}", fwd(dir));
    std::fs::write(&path, password).unwrap();
    path
}

#[test]
#[ignore = "braucht Java 25 und den Interop-Harness (Stufe vault-interop, fixture interop)"]
fn vault_task_create_then_cryptofs_reads() {
    let harness = Harness::from_env();
    let dir = tempfile::tempdir().expect("temporary folder");
    let local = LocalBackend::new("/");
    let root = format!("{}/se-tresor", fwd(dir.path()));
    let config = create_vault(&local, &root, PASSWORD, 220, None).expect("created by se");
    assert_eq!(config.combo, CipherCombo::SivGcm);
    let session = session_for(SITE, &local, &root, PASSWORD).expect("se unlocks it");
    let vault = VaultBackend::new(Arc::new(LocalBackend::new("/")), &root, session);
    let files = files();
    for (path, bytes) in &files {
        if let Some((parent, _)) = path.rsplit_once('/') {
            if !parent.is_empty() {
                vault.mkdir_all(parent).expect("folders");
            }
        }
        write_with(&vault, path, bytes);
    }
    let manifest = manifest(dir.path(), &files);
    let password = password_file(dir.path(), "password.txt", PASSWORD);
    let wrong = password_file(dir.path(), "wrong.txt", "falsch-falsch-falsch");
    // CryptoFS reads every file byte for byte, finds nothing in its health
    // check and refuses a wrong password.
    harness.expect(
        0,
        &[
            "read",
            "--vault",
            &root,
            "--password-file",
            &password,
            "--expect",
            &manifest,
        ],
    );
    harness.expect(
        0,
        &["health", "--vault", &root, "--password-file", &password],
    );
    harness.expect(2, &["unlock", "--vault", &root, "--password-file", &wrong]);
}

#[test]
#[ignore = "braucht Java 25 und den Interop-Harness (Stufe vault-interop, fixture interop)"]
fn vault_task_password_change_then_cryptofs_unlocks_new() {
    let harness = Harness::from_env();
    let dir = tempfile::tempdir().expect("temporary folder");
    let local = LocalBackend::new("/");
    let source = dir.path().join("klartext");
    let files = files();
    for (path, bytes) in &files {
        let target = source.join(path.trim_start_matches('/'));
        std::fs::create_dir_all(target.parent().expect("parent")).unwrap();
        std::fs::write(target, bytes).unwrap();
    }
    let source = fwd(&source);
    let root = format!("{}/cryptofs-tresor", fwd(dir.path()));
    let old = password_file(dir.path(), "alt.txt", PASSWORD);
    let new = password_file(dir.path(), "neu.txt", NEW);
    let manifest = manifest(dir.path(), &files);
    // Cryptomator creates the vault, se changes its password.
    harness.expect(
        0,
        &[
            "create",
            "--vault",
            &root,
            "--password-file",
            &old,
            "--from",
            &source,
        ],
    );
    let changed = change_password(&local, &root, None, PASSWORD, NEW, None).expect("changed by se");
    assert!(std::path::Path::new(&root).join(&changed.backup).exists());
    // CryptoFS unlocks with the new password only and reads every file.
    harness.expect(0, &["unlock", "--vault", &root, "--password-file", &new]);
    harness.expect(2, &["unlock", "--vault", &root, "--password-file", &old]);
    harness.expect(
        0,
        &[
            "read",
            "--vault",
            &root,
            "--password-file",
            &new,
            "--expect",
            &manifest,
        ],
    );
    harness.expect(0, &["health", "--vault", &root, "--password-file", &new]);
    // se reads them with the new password as well.
    let session = session_for(SITE, &local, &root, NEW).expect("se unlocks with the new password");
    let vault = VaultBackend::new(Arc::new(LocalBackend::new("/")), &root, session);
    for (path, bytes) in &files {
        assert_eq!(&read_with(&vault, path), bytes, "{path}");
    }
}
