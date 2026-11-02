//! Writing into the vault: long names, stage names, locking (VLT-M2, stage
//! `vault_unit`).
use std::io::{self, Read, Write};
use std::path::Path;

use crate::cryptomator::{shortened_name, CipherCombo};
use crate::vault::core::locked_site;
use crate::vfs::{self, Backend, VfsMeta};

use super::backend_test_support::{read_with, TestVault, SITE};

fn names(entries: &[VfsMeta]) -> Vec<String> {
    let mut names: Vec<String> = entries.iter().map(|meta| meta.name.clone()).collect();
    names.sort();
    names
}

fn err<T>(result: io::Result<T>) -> io::Error {
    match result {
        Ok(_) => panic!("an error was expected"),
        Err(error) => error,
    }
}

fn write_stage(backend: &dyn Backend, path: &str, data: &[u8]) {
    let mut writer = backend.open_write_copy_stage(path).unwrap();
    writer.write_all(data).unwrap();
    writer.flush().unwrap();
}

#[test]
fn vault_task_backend_long_names_create_rename_replace() {
    let vault = TestVault::new(CipherCombo::SivGcm, 220);
    let backend = &vault.backend;
    let content = vault.content("/");
    let long = format!("{}.txt", "langer Name ".repeat(14));
    let long_path = format!("/{long}");
    vault.write(&long_path, b"eins");
    let encrypted = vault.encrypted("/", &long);
    assert!(encrypted.len() > 220, "{}", encrypted.len());
    let folder = shortened_name(&encrypted);
    assert_eq!(
        vault.stored(&format!("{content}/{folder}")),
        ["contents.c9r", "name.c9s"]
    );
    let inflated = std::fs::read_to_string(vault.at(&format!("{content}/{folder}/name.c9s")));
    assert_eq!(inflated.unwrap(), encrypted);
    assert_eq!(vault.read(&long_path), b"eins");
    assert_eq!(names(&backend.list_dir("/").unwrap()), [long.clone()]);
    // Replace: published into the long-named folder; nothing else left.
    vault.write(&long_path, b"zwei");
    assert_eq!(vault.read(&long_path), b"zwei");
    let stage = vfs::unique_staging_path(backend, &long_path, "copy").unwrap();
    let mut writer = backend.open_write_copy_stage_sized(&stage, 4).unwrap();
    writer.write_all(b"drei").unwrap();
    writer.flush().unwrap();
    drop(writer);
    backend.promote_staged(&stage, &long_path).unwrap();
    assert_eq!(vault.read(&long_path), b"drei");
    assert_eq!(vault.stored(&content), [folder.clone()]);
    // long → long, long → short, short → long.
    let other = format!("{}.md", "anderer langer Name ".repeat(9));
    backend.rename(&long_path, &format!("/{other}")).unwrap();
    assert_eq!(vault.read(&format!("/{other}")), b"drei");
    assert_eq!(
        err(backend.stat(&long_path)).kind(),
        io::ErrorKind::NotFound
    );
    assert!(!Path::new(&vault.at(&format!("{content}/{folder}"))).exists());
    backend.rename(&format!("/{other}"), "/kurz.txt").unwrap();
    assert_eq!(vault.read("/kurz.txt"), b"drei");
    assert_eq!(vault.stored(&content), [vault.encrypted("/", "kurz.txt")]);
    backend.rename("/kurz.txt", &long_path).unwrap();
    assert_eq!(vault.read(&long_path), b"drei");
    assert_eq!(vault.stored(&content), [folder.clone()]);
    // A long-named folder keeps its content through renames.
    let long_dir = "Ordner mit langem Namen ".repeat(8);
    backend.create_dir(&format!("/{long_dir}")).unwrap();
    vault.write(&format!("/{long_dir}/innen.txt"), b"innen");
    backend.rename(&format!("/{long_dir}"), "/Ordner").unwrap();
    assert_eq!(vault.read("/Ordner/innen.txt"), b"innen");
    backend.rename("/Ordner", &format!("/{long_dir}")).unwrap();
    assert_eq!(vault.read(&format!("/{long_dir}/innen.txt")), b"innen");
    let mut expected = vec![long.clone(), long_dir.clone()];
    expected.sort();
    assert_eq!(names(&backend.list_dir("/").unwrap()), expected);
    // Existing destinations are never replaced by a rename.
    vault.write("/a.txt", b"a");
    assert_eq!(
        err(backend.rename("/a.txt", &long_path)).kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(vault.read(&long_path), b"drei");
}

#[test]
fn vault_task_backend_every_stage_form_mapped_invisible() {
    let vault = TestVault::new(CipherCombo::SivGcm, 220);
    let backend = &vault.backend;
    backend.create_dir("/Docs").unwrap();
    let hex16 = "0123456789abcdef";
    let long = "x".repeat(200);
    // Every form of `vfs/core/staging_names.rs`.
    let forms = [
        format!("report.txt.se-copy-{hex16}"),
        format!("{long}.se-mount-{hex16}"),
        ".report.txt.smart-explorer.part".to_string(),
        ".report.txt.smart-explorer-1a2b.part".to_string(),
        ".report.txt.smart-explorer-1a-2b.move".to_string(),
        "report.txt.se-agent-batch-1a-2b.part".to_string(),
        "report.txt.se-upload-1234-abcd.part".to_string(),
        ".se-agent-get-1a-2b-3c.spool".to_string(),
        format!(".se-private-{hex16}{hex16}.tmp"),
        format!(".se-replace-{hex16}"),
    ];
    for form in &forms {
        assert!(vfs::is_staging_name(form), "{form}");
        let path = format!("/Docs/{form}");
        write_stage(backend, &path, b"stufe");
        assert_eq!(backend.stat(&path).unwrap().size, 5, "{form}");
        assert_eq!(vault.read(&path), b"stufe", "{form}");
    }
    let content = vault.content("/Docs");
    let stored: Vec<String> = vault
        .stored(&content)
        .into_iter()
        .filter(|name| name != "dirid.c9r")
        .collect();
    assert_eq!(stored.len(), forms.len());
    for name in &stored {
        for suffix in [".c9r", ".c9s", ".c9u"] {
            assert!(!name.ends_with(suffix), "{name}");
        }
        assert!(name.len() <= 255, "{name}");
        assert!(vfs::is_staging_name(name), "{name}");
        assert!(!name.contains("report") && !name.contains(&long), "{name}");
    }
    // The 200-character file part took the 28-character digest form.
    assert!(stored
        .iter()
        .any(|name| name.len() == 28 + ".se-mount-".len() + 16));
    // Invisible to the listing, also to the tolerant one.
    assert!(backend.list_dir("/Docs").unwrap().is_empty());
    let tolerant = vfs::list_dir_tolerant(backend, "/Docs").unwrap();
    assert!(tolerant.entries.is_empty() && tolerant.omitted.is_empty());
    // Published like any write, then visible under its name.
    let stage = format!("/Docs/report.txt.se-copy-{hex16}");
    backend.promote_staged(&stage, "/Docs/report.txt").unwrap();
    assert_eq!(names(&backend.list_dir("/Docs").unwrap()), ["report.txt"]);
    assert_eq!(vault.read("/Docs/report.txt"), b"stufe");
    // Discarded through the storage.
    backend
        .discard_copy_stage("/Docs/.report.txt.smart-explorer.part")
        .unwrap();
    let left = vault.stored(&content).len();
    assert_eq!(
        left,
        forms.len() - 2 + 1 + 1,
        "{:?}",
        vault.stored(&content)
    );
    // A name that is no stage cannot be a stage.
    assert_eq!(
        err(backend.open_write_copy_stage("/Docs/plain.txt")).kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn vault_task_backend_locked_fails_and_publishes_nothing() {
    let vault = TestVault::new(CipherCombo::SivGcm, 220);
    let backend = &vault.backend;
    vault.write("/alt.txt", b"alt");
    let content = vault.content("/");
    let stored_old = vault.encrypted("/", "alt.txt");
    let mut writer = backend.open_write("/neu.txt").unwrap();
    writer.write_all(b"neu").unwrap();
    let mut replacing = backend.open_write("/alt.txt").unwrap();
    replacing.write_all(b"ersetzt").unwrap();
    let mut reader = backend.open_read("/alt.txt").unwrap();
    assert!(!backend.cache.is_empty());
    vault.session.lock();
    let locked = |error: io::Error| {
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        assert_eq!(locked_site(&error), Some(SITE), "{error}");
    };
    // Streams already open fail, and nothing is published.
    locked(writer.flush().unwrap_err());
    locked(replacing.flush().unwrap_err());
    locked(reader.read(&mut [0u8; 4]).unwrap_err());
    // Every operation fails; locked is never „absent“.
    locked(err(backend.try_exists("/alt.txt")));
    locked(err(backend.stat("/alt.txt")));
    locked(err(backend.list_dir("/")));
    locked(err(backend.open_read("/alt.txt")));
    locked(err(backend.open_write("/x.txt")));
    locked(err(backend.create_dir("/D")));
    locked(err(backend.rename("/alt.txt", "/b.txt")));
    locked(err(backend.remove_file("/alt.txt")));
    locked(err(vfs::list_dir_tolerant(backend, "/")));
    // Cleartext paths are gone from memory (K46).
    assert!(backend.cache.is_empty());
    drop(writer);
    drop(replacing);
    // The old content stays, the new file never appeared, no stage is left.
    let reopened = vault.reopened();
    assert_eq!(read_with(&reopened, "/alt.txt"), b"alt");
    assert_eq!(
        err(reopened.stat("/neu.txt")).kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(vault.stored(&content), [stored_old]);
}
