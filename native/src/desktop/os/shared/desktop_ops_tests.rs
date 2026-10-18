//! DSK-M4: rename, recycle bin, copy and move in a temporary desktop folder,
//! through the existing paths (local VFS, `trash`, copy engine) and never
//! outside the folder.
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::{check_name, new_folder, recycle, rename, transfer, NEW_FOLDER};
use crate::types::CopyMode;

fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn desktop_task_desktop_ops_rename_trash_copy() {
    let root = tempfile::tempdir().unwrap();
    let desktop = root.path().join("Desktop");
    std::fs::create_dir(&desktop).unwrap();
    std::fs::write(desktop.join("a.txt"), b"alpha").unwrap();
    std::fs::write(desktop.join("b.txt"), b"beta").unwrap();
    std::fs::write(root.path().join("draussen.txt"), b"bleibt").unwrap();

    // Umbenennen: atomic, never replacing, one entry of the folder only.
    let renamed = rename(&desktop, "a.txt", "c.txt").unwrap();
    assert_eq!(renamed, desktop.join("c.txt"));
    assert_eq!(std::fs::read(&renamed).unwrap(), b"alpha");
    let taken = rename(&desktop, "c.txt", "b.txt").unwrap_err();
    assert!(taken.contains("gibt es schon"), "{taken}");
    assert_eq!(std::fs::read(desktop.join("b.txt")).unwrap(), b"beta");
    for bad in [
        "",
        " ",
        ".",
        "..",
        "../x.txt",
        "unter/name",
        "rück\\weg",
        "zeile\nzwei",
    ] {
        assert!(check_name(bad).is_err(), "{bad:?}");
        assert!(rename(&desktop, "c.txt", bad).is_err(), "{bad:?}");
    }
    assert!(rename(&desktop, "../draussen.txt", "drinnen.txt").is_err());
    assert_eq!(names(&desktop), ["b.txt", "c.txt"]);

    // Neuer Ordner: the first free name.
    assert_eq!(new_folder(&desktop).unwrap(), desktop.join(NEW_FOLDER));
    assert_eq!(
        new_folder(&desktop).unwrap(),
        desktop.join(format!("{NEW_FOLDER} (2)"))
    );
    assert!(desktop.join(NEW_FOLDER).is_dir());

    // In Papierkorb: each entry to the recycle bin; a refused name or a
    // failing bin never deletes anything.
    let bin = root.path().join("Papierkorb");
    std::fs::create_dir(&bin).unwrap();
    let recycled: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    let to_bin = |path: &Path| -> Result<(), String> {
        let name = path.file_name().ok_or("kein Name")?;
        std::fs::rename(path, bin.join(name)).map_err(|error| error.to_string())?;
        recycled.lock().unwrap().push(path.to_path_buf());
        Ok(())
    };
    let failed = recycle(
        &desktop,
        &["b.txt".to_string(), "../draussen.txt".to_string()],
        &to_bin,
    );
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].0, "../draussen.txt");
    assert_eq!(*recycled.lock().unwrap(), [desktop.join("b.txt")]);
    assert!(bin.join("b.txt").is_file());
    assert!(root.path().join("draussen.txt").is_file());
    let refuse = |_: &Path| -> Result<(), String> { Err("Papierkorb voll".into()) };
    let failed = recycle(&desktop, &["c.txt".to_string()], &refuse);
    assert_eq!(
        failed,
        [("c.txt".to_string(), "Papierkorb voll".to_string())]
    );
    assert!(
        desktop.join("c.txt").is_file(),
        "never deleted without the bin"
    );

    // Kopieren and Einfügen with the copy engine: names and trees kept, a
    // taken name gets a new one, the sources stay.
    let source = root.path().join("Quelle");
    std::fs::create_dir_all(source.join("Ordner")).unwrap();
    std::fs::write(source.join("doc.txt"), b"dokument").unwrap();
    std::fs::write(source.join("Ordner/innen.txt"), b"innen").unwrap();
    let sources = [source.join("doc.txt"), source.join("Ordner")];
    let report = transfer(&sources, &desktop, CopyMode::Copy);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(std::fs::read(desktop.join("doc.txt")).unwrap(), b"dokument");
    assert_eq!(
        std::fs::read(desktop.join("Ordner/innen.txt")).unwrap(),
        b"innen"
    );
    assert!(source.join("doc.txt").is_file());
    let report = transfer(&sources[..1], &desktop, CopyMode::Copy);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let copies = names(&desktop)
        .into_iter()
        .filter(|name| name.starts_with("doc") && name.ends_with(".txt"))
        .count();
    assert_eq!(copies, 2, "{:?}", names(&desktop));
    assert_eq!(std::fs::read(desktop.join("doc.txt")).unwrap(), b"dokument");

    // Ausschneiden and Einfügen moves; moving within the folder is no change.
    std::fs::write(source.join("weg.txt"), b"weg").unwrap();
    let report = transfer(&[source.join("weg.txt")], &desktop, CopyMode::Move);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(!source.join("weg.txt").exists());
    assert_eq!(std::fs::read(desktop.join("weg.txt")).unwrap(), b"weg");
    let before = names(&desktop);
    let report = transfer(&[desktop.join("weg.txt")], &desktop, CopyMode::Move);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(names(&desktop), before);
}
