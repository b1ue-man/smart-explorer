//! Language packs: add, remove, precedence, invalid packs, cache.
use std::fs;
use std::path::Path;

use crate::textview::{LanguageStore, PackOrigin, PackSource, TextViewError};

const MEIN_RUST: &str = "%YAML 1.2\n---\nname: MeinRust\nfile_extensions: [rs, meinrs]\nscope: source.meinrust\ncontexts:\n  main:\n    - match: '\\bfn\\b'\n      scope: keyword.other.mein\n";

const UNRESOLVED: &str = "%YAML 1.2\n---\nname: Verweis\nfile_extensions: [verweis]\nscope: source.verweis\ncontexts:\n  main:\n    - match: 'x'\n      push: scope:source.gibtsnicht\n";

fn store(root: &Path) -> LanguageStore {
    LanguageStore::new(root.join("languages"), root.join("cache"))
}

fn bytes(file_name: &str, text: &str) -> PackSource {
    PackSource::Bytes {
        file_name: file_name.to_string(),
        bytes: text.as_bytes().to_vec(),
    }
}

fn dumps(cache: &Path) -> usize {
    fs::read_dir(cache)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".packdump"))
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn textview_task_pack_add_remove_precedence() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path());
    let before = store.fingerprint();
    // Built-in entries carry origin and a license note.
    let builtin = store.list();
    let rust = builtin
        .iter()
        .find(|p| p.name == "Rust")
        .expect("Rust built in");
    assert_eq!(rust.origin, PackOrigin::Builtin);
    assert!(rust.license.contains("Rust"), "{}", rust.license);
    assert!(rust.languages[0].extensions.contains(&"rs".to_string()));
    assert!(builtin.iter().all(|p| !p.license.is_empty()));
    assert_eq!(store.highlighter().detect("main.rs", ""), "Rust");

    let added = store
        .add(bytes("mein.sublime-syntax", MEIN_RUST), None)
        .unwrap();
    assert_eq!(added.name, "mein");
    assert_eq!(added.origin, PackOrigin::Added);
    assert_eq!(added.languages[0].name, "MeinRust");
    assert!(store.folder().join("mein.sublime-syntax").is_file());
    assert_ne!(store.fingerprint(), before);
    assert_eq!(dumps(store.cache_folder()), 1, "add keeps the built set");

    let list = store.list();
    assert_eq!(list[0].name, "mein");
    let rust = list.iter().find(|p| p.name == "Rust").unwrap();
    assert_eq!(rust.shadowed_by.as_deref(), Some("mein"));
    // The added pack wins the extension; its own extension works too.
    let highlighter = store.highlighter();
    assert_eq!(highlighter.detect("main.rs", ""), "MeinRust");
    assert_eq!(highlighter.detect("x.meinrs", ""), "MeinRust");
    assert!(highlighter.problems().is_empty());
    assert_eq!(dumps(store.cache_folder()), 1);

    assert!(matches!(
        store.add(bytes("mein.sublime-syntax", MEIN_RUST), None),
        Err(TextViewError::PackExists(name)) if name == "mein"
    ));
    assert!(matches!(
        store.remove("Rust"),
        Err(TextViewError::BuiltinPack(_))
    ));
    assert!(matches!(
        store.remove("gibtsnicht"),
        Err(TextViewError::PackNotFound(_))
    ));
    store.remove("mein").unwrap();
    assert!(!store.folder().join("mein.sublime-syntax").exists());
    assert_eq!(store.highlighter().detect("main.rs", ""), "Rust");
}

#[test]
fn textview_task_pack_folder() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path());
    let source = root.path().join("MeinPaket");
    fs::create_dir_all(source.join("Syntaxes")).unwrap();
    fs::write(source.join("Syntaxes/Mein.sublime-syntax"), MEIN_RUST).unwrap();
    fs::write(source.join("LICENSE"), "MIT").unwrap();
    fs::write(source.join("Mein.tmPreferences"), "<plist/>").unwrap();
    let added = store.add(PackSource::Path(source), None).unwrap();
    assert_eq!(added.name, "MeinPaket");
    assert!(added.license.contains("LICENSE"), "{}", added.license);
    let installed = store.folder().join("MeinPaket");
    assert!(installed.join("Syntaxes/Mein.sublime-syntax").is_file());
    assert!(installed.join("LICENSE").is_file());
    assert!(!installed.join("Mein.tmPreferences").exists());
    // A file pack under an explicit name.
    let file = root.path().join("andere.sublime-syntax");
    fs::write(&file, MEIN_RUST.replace("MeinRust", "Andere")).unwrap();
    let named = store.add(PackSource::Path(file), Some("Zweite")).unwrap();
    assert_eq!(named.name, "Zweite");
    store.remove("MeinPaket").unwrap();
    assert!(!installed.exists());
}

#[test]
fn textview_task_pack_invalid() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path());
    let broken = store.add(bytes("kaputt.sublime-syntax", "name: x\ncontexts: ["), None);
    match broken {
        Err(TextViewError::InvalidPack(reason)) => {
            assert!(reason.contains("keine gültige Sublime-Syntax"), "{reason}")
        }
        other => panic!("expected InvalidPack, got {other:?}"),
    }
    assert!(matches!(
        store.add(bytes("notiz.txt", MEIN_RUST), None),
        Err(TextViewError::InvalidPack(_))
    ));
    assert!(matches!(
        store.add(bytes("mein.sublime-syntax", MEIN_RUST), Some("../weg")),
        Err(TextViewError::InvalidPackName(_))
    ));
    let empty = root.path().join("leer");
    fs::create_dir_all(&empty).unwrap();
    assert!(matches!(
        store.add(PackSource::Path(empty), None),
        Err(TextViewError::InvalidPack(_))
    ));
    match store.add(bytes("verweis.sublime-syntax", UNRESOLVED), None) {
        Err(TextViewError::InvalidPack(reason)) => assert!(reason.contains("verweist"), "{reason}"),
        other => panic!("expected InvalidPack, got {other:?}"),
    }
    assert!(!store.folder().join("verweis.sublime-syntax").exists());
    // A broken file put into the folder by hand is skipped and reported.
    fs::create_dir_all(store.folder()).unwrap();
    fs::write(store.folder().join("hand.sublime-syntax"), "name: [").unwrap();
    store
        .add(bytes("mein.sublime-syntax", MEIN_RUST), None)
        .unwrap();
    let highlighter = store.highlighter();
    assert_eq!(highlighter.detect("a.meinrs", ""), "MeinRust");
    assert_eq!(highlighter.problems().len(), 1);
    assert!(highlighter.problems()[0].contains("hand"));
    let hand = store
        .list()
        .into_iter()
        .find(|p| p.name == "hand")
        .expect("listed with its problem");
    assert_eq!(hand.problems.len(), 1);
    assert!(hand.languages.is_empty());
}
