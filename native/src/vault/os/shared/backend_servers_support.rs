//! The scenario of the vault tests on real servers (VLT-M14, stage
//! `vault-backends`): a vault is created on the storage, unlocked, written,
//! read, renamed and listed; then the storage itself is searched for the
//! cleartext of every name and content that went in. The servers are the
//! owned ones of `native/sync-reliability-providers.py`, opened through the
//! saved connections and the shared resolution like the sync provider tests
//! do (`SE_SYNC_PROVIDER_MANIFEST`,
//! `connect::sync_reliability_task_provider_fixture`).
use std::io::{self, Read};
use std::sync::Arc;

use crate::connect::sync_reliability_task_provider_fixture as fixture;
use crate::cryptomator::{
    file_size, CipherCombo, CIPHER_SUFFIX, DATA_DIR, MASTERKEY_FILE, SHORTENED_SUFFIX,
    VAULT_CONFIG_FILE,
};
use crate::vault::core::path_map::storage_path;
use crate::vfs::{Backend, BackendHandle, Scheme};

use super::backend::VaultBackend;
use super::backend_test_support::{pattern, read_with, write_with, SITE};
use super::manage::create_vault;
use super::unlock::session_for;

const PASSWORD: &str = "Tresor-Passwort-Server-2026";
/// The vault folder inside the folder a provider opens for a test.
const VAULT_FOLDER: &str = "tresor";
/// Length of the second part of a directory hash folder (`d/<2>/<30>`).
const HASH_REST_LEN: usize = 30;

/// `bytes` random bytes as lowercase hex digits.
pub(super) fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    getrandom::getrandom(&mut buffer).expect("random bytes of the operating system");
    buffer.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The text the tests put into names and contents; it must never reach the
/// storage in the clear.
pub(super) fn new_marker() -> String {
    format!("rdtmark{}", random_hex(10))
}

/// The message of a caught panic.
pub(super) fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else {
        "Panik ohne Text".to_string()
    }
}

/// Ends the test as "not run" when the suite did not start the servers.
pub(super) fn require_servers() {
    if std::env::var_os("SE_SYNC_PROVIDER_MANIFEST").is_none() {
        panic!(
            "nicht ausgeführt: SE_SYNC_PROVIDER_MANIFEST fehlt: die Testserver startet nur die \
             Stufe vault-backends (native/sync-reliability-providers.py)"
        );
    }
}

/// What a storage holds below the vault root: its folders and files (paths
/// relative to the root) and the bytes of every file.
struct Stored {
    folders: Vec<String>,
    files: Vec<(String, Vec<u8>)>,
}

/// Everything below `root` as the server itself serves it.
fn stored_tree(inner: &dyn Backend, root: &str) -> Stored {
    // Nothing the vault wrote may hide behind a listing cache.
    inner.invalidate_cache();
    let mut stored = Stored {
        folders: Vec::new(),
        files: Vec::new(),
    };
    let mut pending = vec![String::new()];
    while let Some(relative) = pending.pop() {
        let folder = storage_path(root, &relative);
        let entries = inner
            .list_dir(&folder)
            .unwrap_or_else(|error| panic!("Speicher auflisten ({folder}): {error}"));
        for entry in entries {
            let below = if relative.is_empty() {
                entry.name.clone()
            } else {
                format!("{relative}/{}", entry.name)
            };
            if entry.is_dir {
                stored.folders.push(below.clone());
                pending.push(below);
                continue;
            }
            let mut bytes = Vec::new();
            if !entry.is_symlink && !entry.special {
                let path = storage_path(root, &below);
                inner
                    .open_read(&path)
                    .and_then(|mut reader| reader.read_to_end(&mut bytes))
                    .unwrap_or_else(|error| panic!("Speicher lesen ({path}): {error}"));
            }
            stored.files.push((below, bytes));
        }
    }
    stored
}

/// The marker as the bytes of a file or name can hold it: UTF-8 and UTF-16LE.
fn needles(marker: &str) -> [Vec<u8>; 2] {
    [
        marker.as_bytes().to_vec(),
        marker.encode_utf16().flat_map(u16::to_le_bytes).collect(),
    ]
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// Whether `parts` (a path below the vault root) is a place of the vault
/// format: the two key files and `d`, the two levels of directory hash
/// folders, and below them only `.c9r` and `.c9s` names.
fn vault_shape(parts: &[&str], folder: bool) -> bool {
    match parts {
        [top] if folder => *top == DATA_DIR,
        [top] => *top == MASTERKEY_FILE || *top == VAULT_CONFIG_FILE,
        [data, prefix] => folder && *data == DATA_DIR && prefix.len() == 2,
        [data, prefix, rest] => {
            folder && *data == DATA_DIR && prefix.len() == 2 && rest.len() == HASH_REST_LEN
        }
        [data, .., name] => {
            *data == DATA_DIR && (name.ends_with(CIPHER_SUFFIX) || name.ends_with(SHORTENED_SUFFIX))
        }
        _ => false,
    }
}

/// The storage holds the vault format and nothing in the clear: no name and
/// no byte carries the marker, only the places of the format exist, and the
/// big file lies there as ciphertext of the size the format gives it.
fn assert_only_ciphertext(stored: &Stored, marker: &str, clear_len: usize) {
    let needles = needles(marker);
    let paths = || {
        stored
            .folders
            .iter()
            .chain(stored.files.iter().map(|(path, _)| path))
    };
    for path in paths() {
        for part in path.split('/') {
            assert!(
                !needles
                    .iter()
                    .any(|needle| contains(part.as_bytes(), needle)),
                "Klartext im gespeicherten Namen: {path}"
            );
        }
    }
    for path in &stored.folders {
        let parts: Vec<&str> = path.split('/').collect();
        assert!(
            vault_shape(&parts, true),
            "unerwarteter Ordner im Tresorspeicher: {path}"
        );
    }
    for (path, _) in &stored.files {
        let parts: Vec<&str> = path.split('/').collect();
        assert!(
            vault_shape(&parts, false),
            "unerwartete Datei im Tresorspeicher: {path}"
        );
    }
    let mut top: Vec<&str> = paths()
        .filter(|path| !path.contains('/'))
        .map(String::as_str)
        .collect();
    top.sort_unstable();
    assert_eq!(
        top,
        [DATA_DIR, MASTERKEY_FILE, VAULT_CONFIG_FILE],
        "oberste Ebene des Tresorspeichers"
    );
    let mut total = 0usize;
    for (path, bytes) in &stored.files {
        total += bytes.len();
        assert!(
            !needles.iter().any(|needle| contains(bytes, needle)),
            "Klartext im Inhalt der gespeicherten Datei {path}"
        );
    }
    assert!(
        total >= clear_len,
        "der Speicher liefert weniger Bytes ({total}) als die Datei groß ist ({clear_len})"
    );
    let whole = file_size(clear_len as u64, CipherCombo::SivGcm);
    assert!(
        stored
            .files
            .iter()
            .any(|(_, bytes)| bytes.len() as u64 == whole),
        "keine gespeicherte Datei hat die verschlüsselte Größe {whole}"
    );
}

/// Compares bytes without printing them: a mismatch names the sizes and the
/// first difference.
fn assert_same_bytes(actual: &[u8], expected: &[u8], what: &str) {
    if actual == expected {
        return;
    }
    let first = actual
        .iter()
        .zip(expected)
        .position(|(left, right)| left != right);
    panic!(
        "{what}: {} Bytes statt {}, erster Unterschied bei {first:?}",
        actual.len(),
        expected.len()
    );
}

/// The sorted names in a folder of the vault.
fn names_in(vault: &VaultBackend, path: &str) -> Vec<String> {
    let mut names: Vec<String> = vault
        .list_dir(path)
        .unwrap_or_else(|error| panic!("auflisten {path}: {error}"))
        .into_iter()
        .map(|meta| meta.name)
        .collect();
    names.sort();
    names
}

/// The scenario on the storage `inner`, below its folder `base`: create the
/// vault, unlock it, make folders, write, read, rename and list, unlock it
/// again from what the server holds, then search the server's own view of
/// the vault folder for the marker. Panics with the first difference.
pub(super) fn exercise(inner: &BackendHandle, base: &str, marker: &str) {
    assert!(
        !matches!(inner.scheme(), Scheme::Local | Scheme::Vault),
        "der Speicher ist kein Server: {:?}",
        inner.scheme()
    );
    let root = fixture::path(&**inner, base, VAULT_FOLDER);
    let config = create_vault(&**inner, &root, PASSWORD, 220, None)
        .unwrap_or_else(|error| panic!("Tresor anlegen: {error}"));
    assert_eq!(config.combo, CipherCombo::SivGcm);
    let session = session_for(SITE, &**inner, &root, PASSWORD)
        .unwrap_or_else(|error| panic!("Tresor entsperren: {error}"));
    let vault = VaultBackend::new(Arc::clone(inner), &root, session);

    let folder = format!("Ordner-{marker}");
    let nested = format!("Unter-{marker}");
    let file = format!("Übung-{marker}.txt");
    let renamed = format!("Neu-{marker}.txt");
    let top = format!("Wurzel-{marker}.txt");
    // Marker at both ends; 70 000 bytes between them are three chunks.
    let mut content = format!("Anfang {marker}\n").into_bytes();
    content.extend(pattern(70_000));
    content.extend(format!("\nEnde {marker}\n").into_bytes());

    vault
        .mkdir_all(&format!("/{folder}/{nested}"))
        .unwrap_or_else(|error| panic!("Ordner anlegen: {error}"));
    let path = format!("/{folder}/{file}");
    write_with(&vault, &path, &content);
    assert_same_bytes(&read_with(&vault, &path), &content, "gelesene Bytes");
    let renamed_path = format!("/{folder}/{renamed}");
    vault
        .rename(&path, &renamed_path)
        .unwrap_or_else(|error| panic!("umbenennen: {error}"));
    assert_same_bytes(
        &read_with(&vault, &renamed_path),
        &content,
        "Bytes nach dem Umbenennen",
    );
    let gone = vault.stat(&path).err().map(|error| error.kind());
    assert_eq!(
        gone,
        Some(io::ErrorKind::NotFound),
        "der alte Name nach dem Umbenennen"
    );
    write_with(&vault, &format!("/{top}"), marker.as_bytes());

    let mut in_folder = vec![renamed.clone(), nested.clone()];
    in_folder.sort();
    assert_eq!(names_in(&vault, &format!("/{folder}")), in_folder);
    let mut in_root = vec![folder.clone(), top.clone()];
    in_root.sort();
    assert_eq!(names_in(&vault, "/"), in_root);
    let listing = vault
        .list_dir(&format!("/{folder}"))
        .unwrap_or_else(|error| panic!("auflisten: {error}"));
    let file_meta = listing
        .iter()
        .find(|meta| meta.name == renamed)
        .expect("die umbenannte Datei steht in der Liste");
    assert!(!file_meta.is_dir);
    assert_eq!(file_meta.size, content.len() as u64, "Klartextgröße");
    let folder_meta = listing
        .iter()
        .find(|meta| meta.name == nested)
        .expect("der Unterordner steht in der Liste");
    assert!(folder_meta.is_dir);

    // A new unlock from what the server holds sees the same vault.
    let again = session_for(SITE, &**inner, &root, PASSWORD)
        .unwrap_or_else(|error| panic!("Tresor erneut entsperren: {error}"));
    let second = VaultBackend::new(Arc::clone(inner), &root, again);
    assert_same_bytes(
        &read_with(&second, &renamed_path),
        &content,
        "Bytes nach dem erneuten Entsperren",
    );
    assert_eq!(names_in(&second, "/"), in_root);

    let stored = stored_tree(&**inner, &root);
    assert_only_ciphertext(&stored, marker, content.len());
}
