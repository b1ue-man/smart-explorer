//! „Tresor prüfen“ (stage `vault_compare`, vault plan C6.2, C6.6): every
//! finding of CryptoFS 2.11.0 and of se found in a prepared vault, the
//! fixes (the orphan below `LOST+FOUND`, the stage that goes) and the copy of
//! the cipher files before a removal.
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::compare_test_support::{stored, Fx};
use super::dir_map::DirMap;
use super::health::{check_site_in, CheckResult};
use super::session_test_support::local_resolve;
use crate::vault::core::compare_model::{CompareGroup, CompareItem, ItemAction};
use crate::vault::core::health_model::FindingKind;
use crate::vfs::LocalBackend;

/// A vault with two folders, a file at the top and a long name.
fn vault() -> Fx {
    let fx = Fx::bare();
    fx.write(&fx.root_a, "/Ordner/x.txt", b"x");
    fx.write(&fx.root_a, "/Zwei/y.txt", b"y");
    fx.write(&fx.root_a, "/Bericht.txt", b"b");
    fx.write(&fx.root_a, &long_name(), b"lang");
    fx
}

fn long_name() -> String {
    format!("/{}.txt", "q".repeat(170))
}

fn map(fx: &Fx) -> DirMap {
    DirMap::build(
        &LocalBackend::new("/"),
        &fx.root_a,
        &fx.session,
        &AtomicBool::new(false),
    )
    .expect("map")
}

fn check(fx: &Fx) -> CheckResult {
    check_site_in(
        &fx.store,
        &local_resolve,
        &fx.site_a.id,
        &AtomicBool::new(false),
    )
    .expect("check")
}

fn names(items: &[CompareItem]) -> Vec<String> {
    let mut found: Vec<String> = items
        .iter()
        .filter_map(|item| item.finding.clone())
        .collect();
    found.sort();
    found
}

fn path_of(fx: &Fx, rel: &str) -> PathBuf {
    Path::new(&fx.root_a).join(rel)
}

/// The stored entry (`d/XX/YYYY/NAME`) of a cleartext path.
fn entry_rel(map: &DirMap, clear: &str) -> String {
    map.names
        .iter()
        .find(|(_, info)| info.clear == clear)
        .map(|(rel, _)| rel.clone())
        .unwrap_or_else(|| panic!("no entry {clear}"))
}

/// The findings the tests prepare: every one of CryptoFS 2.11.0 except the
/// three healthy ones, and the four of se that need a prepared vault.
const PREPARED: [FindingKind; 17] = [
    FindingKind::DirIdCollision,
    FindingKind::EmptyDirFile,
    FindingKind::ObeseDirFile,
    FindingKind::LooseDirFile,
    FindingKind::MissingContentDir,
    FindingKind::MissingDirIdBackup,
    FindingKind::OrphanContentDir,
    FindingKind::LongShortNamesMismatch,
    FindingKind::MissingLongName,
    FindingKind::NotDecodableLongName,
    FindingKind::ObeseNameFile,
    FindingKind::TrailingBytesInNameFile,
    FindingKind::UnknownType,
    FindingKind::AmbiguousType,
    FindingKind::UndecryptableName,
    FindingKind::ForeignConflictCopy,
    FindingKind::StageLeftover,
];

fn name_file(fx: &Fx, map: &DirMap) -> PathBuf {
    let long = entry_rel(map, &long_name());
    path_of(fx, &format!("{long}/name.c9s"))
}

/// Spoils the vault so that the check finds `kind`.
fn spoil(kind: FindingKind, fx: &Fx, map: &DirMap) {
    let node_file = |clear: &str| path_of(fx, &format!("{}/dir.c9r", entry_rel(map, clear)));
    let foreign = |suffix: &str| {
        path_of(
            fx,
            &format!("{}/{}.c9r{suffix}", map.by_clear["/"], "A".repeat(48)),
        )
    };
    match kind {
        FindingKind::DirIdCollision => {
            let one = std::fs::read(node_file("/Ordner")).expect("dir.c9r");
            std::fs::write(node_file("/Zwei"), one).expect("same ID");
        }
        FindingKind::EmptyDirFile => std::fs::write(node_file("/Ordner"), b"").expect("empty"),
        FindingKind::ObeseDirFile => {
            std::fs::write(node_file("/Ordner"), vec![b'x'; 2000]).expect("obese");
        }
        FindingKind::LooseDirFile => {
            let loose = path_of(fx, &format!("{}/dir.c9r", map.by_clear["/"]));
            std::fs::write(loose, b"id").expect("loose");
        }
        FindingKind::MissingContentDir => {
            std::fs::remove_dir_all(path_of(fx, &map.by_clear["/Ordner"])).expect("removed");
        }
        FindingKind::MissingDirIdBackup => {
            let backup = format!("{}/dirid.c9r", map.by_clear["/Ordner"]);
            std::fs::remove_file(path_of(fx, &backup)).expect("removed");
        }
        FindingKind::OrphanContentDir => {
            std::fs::remove_dir_all(path_of(fx, &entry_rel(map, "/Zwei"))).expect("node removed");
        }
        FindingKind::LongShortNamesMismatch => {
            let other = entry_rel(map, "/Bericht.txt");
            let name = other.rsplit('/').next().expect("name").to_string();
            std::fs::write(name_file(fx, map), name).expect("other name");
        }
        FindingKind::MissingLongName => {
            std::fs::remove_file(name_file(fx, map)).expect("removed");
        }
        FindingKind::NotDecodableLongName => {
            std::fs::write(name_file(fx, map), b"kein Name").expect("junk");
        }
        FindingKind::ObeseNameFile => {
            std::fs::write(name_file(fx, map), vec![b'x'; 11 * 1024]).expect("obese");
        }
        FindingKind::TrailingBytesInNameFile => {
            let file = name_file(fx, map);
            let mut text = std::fs::read(&file).expect("name");
            text.extend_from_slice(b"xyz");
            std::fs::write(&file, text).expect("trailing");
        }
        FindingKind::UnknownType => {
            let file = path_of(fx, &entry_rel(map, "/Bericht.txt"));
            std::fs::remove_file(&file).expect("file removed");
            std::fs::create_dir(&file).expect("untyped node");
        }
        FindingKind::AmbiguousType => {
            let node = entry_rel(map, "/Zwei");
            std::fs::write(path_of(fx, &format!("{node}/symlink.c9r")), b"l").expect("two types");
        }
        FindingKind::UndecryptableName => {
            std::fs::write(foreign(""), b"fremd").expect("foreign name");
        }
        FindingKind::ForeignConflictCopy => {
            let rel = entry_rel(map, "/Bericht.txt");
            let base = rel.strip_suffix(".c9r").expect("suffix");
            let copy = path_of(fx, &format!("{base} (1).c9r"));
            std::fs::rename(path_of(fx, &rel), copy).expect("conflict copy");
        }
        FindingKind::StageLeftover => {
            std::fs::write(foreign(".se-compare-0123456789abcdef"), b"halb").expect("stage");
        }
        other => panic!("not prepared: {}", other.name()),
    }
}

#[test]
fn vault_task_check_maps_every_cryptofs_finding() {
    // A healthy vault has no finding: the three healthy ones are never listed.
    let healthy = vault();
    assert!(check(&healthy).items.is_empty());
    drop(healthy);
    for kind in PREPARED {
        let fx = vault();
        spoil(kind, &fx, &map(&fx));
        let result = check(&fx);
        let found = names(&result.items);
        assert!(
            found.iter().any(|name| name == kind.name()),
            "{}: {found:?}",
            kind.name()
        );
        // The entry offers the actions of the table (C6.2) and says why.
        let item = result
            .items
            .iter()
            .find(|item| item.finding.as_deref() == Some(kind.name()))
            .expect("the entry");
        assert_eq!(
            item.actions.len(),
            kind.actions(false).len(),
            "{}: {:?}",
            kind.name(),
            item.actions
        );
        assert_eq!(item.group, CompareGroup::Structure);
        assert!(item.note.is_some());
    }
}

#[test]
fn vault_task_check_orphan_attached_under_lost_found() {
    let fx = vault();
    let before = map(&fx);
    let orphan = before.by_clear["/Zwei"].clone();
    std::fs::remove_dir_all(path_of(&fx, &entry_rel(&before, "/Zwei"))).expect("node removed");
    let result = check(&fx);
    let entry = result
        .items
        .iter()
        .find(|item| item.finding.as_deref() == Some("OrphanContentDir"))
        .expect("the orphan");
    assert_eq!(entry.actions, [ItemAction::AttachOrphan]);
    let report = fx
        .resolve_check(&entry.id, "attach-orphan", false)
        .expect("attached");
    assert_eq!(report.resolved, 1);
    // The entries stay where they are; a node below LOST+FOUND points to them.
    assert!(path_of(&fx, &format!("{orphan}/dirid.c9r")).exists());
    let tree = fx.tree(&fx.root_a);
    let restored: Vec<&String> = tree
        .keys()
        .filter(|path| path.starts_with("/LOST+FOUND/Wiederhergestellt "))
        .collect();
    assert_eq!(restored.len(), 1, "{:?}", tree.keys());
    assert!(restored[0].ends_with("/y.txt"), "{}", restored[0]);
    assert_eq!(tree[restored[0].as_str()], b"y");
    // The check finds nothing of this kind any more.
    assert!(!names(&check(&fx).items)
        .iter()
        .any(|name| name == "OrphanContentDir"));
}

#[test]
fn vault_task_check_stage_leftover_removed() {
    let fx = vault();
    let stage = format!(
        "{}/{}.c9r.se-compare-0123456789abcdef",
        map(&fx).by_clear["/"],
        "A".repeat(48)
    );
    std::fs::write(path_of(&fx, &stage), b"halb").expect("stage");
    let result = check(&fx);
    let entry = result
        .items
        .iter()
        .find(|item| item.finding.as_deref() == Some("StageLeftover"))
        .expect("the stage");
    assert_eq!(entry.actions, [ItemAction::RemoveStage]);
    // A removal is asked about first.
    let asked = fx
        .resolve_check(&entry.id, "remove-stage", false)
        .expect_err("asked");
    assert!(asked.to_string().contains("Kopie des Chiffrats"), "{asked}");
    assert!(path_of(&fx, &stage).exists());
    fx.resolve_check(&entry.id, "remove-stage", true)
        .expect("removed");
    assert!(!path_of(&fx, &stage).exists());
    // A copy of the cipher file waits in the removal folder.
    let kept = super::compare_files::removed_root().join(&fx.site_a.id);
    let copies: Vec<String> = walk(&kept);
    assert!(
        copies.iter().any(|name| name.ends_with("0123456789abcdef")),
        "{copies:?}"
    );
    let _ = std::fs::remove_dir_all(&kept);
    assert!(stored(&fx.root_a)
        .keys()
        .all(|rel| !rel.contains(".se-compare-")));
}

fn walk(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(walk(&path));
        } else {
            found.push(path.to_string_lossy().into_owned());
        }
    }
    found
}
