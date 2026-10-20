//! The actions on the entries of a comparison (stage `vault_compare`, vault
//! plan C6.4): keeping a side, both versions with a valid cipher name,
//! merging as text with ciphertext only in the engine's files, and the
//! checks every action passes. After every action the pair is done: the
//! next run finds nothing left to do. Folders and deletions are in
//! `compare_folders_tests.rs`.
use std::path::Path;
use std::sync::atomic::AtomicBool;

use super::compare_cache::cache;
use super::compare_test_support::{item, stored, tick, Fx};
use crate::vault::core::compare_model::{CompareGroup, ItemAction};

/// A run finds nothing to do, nothing open, and both sides read alike.
fn assert_converged(fx: &Fx) {
    let run = fx.run();
    assert!(run.unavailable.is_none(), "{:?}", run.unavailable);
    assert!(run.outcome.errors.is_empty(), "{:?}", run.outcome.errors);
    assert!(
        run.outcome.conflicts.is_empty(),
        "{:?}",
        run.outcome.conflicts
    );
    assert!(
        run.outcome.deferred.is_empty(),
        "{:?}",
        run.outcome.deferred
    );
    let again = fx.run();
    let stats = &again.outcome.stats;
    assert_eq!((stats.a_to_b, stats.b_to_a, stats.deleted), (0, 0, 0));
    assert_eq!(fx.tree(&fx.root_a), fx.tree(&fx.root_b));
}

/// Both sides changed `/konflikt.txt` (B later than A).
fn conflicting() -> Fx {
    let fx = Fx::new();
    fx.write(&fx.root_a, "/konflikt.txt", b"Fassung von A");
    tick();
    fx.write(&fx.root_b, "/konflikt.txt", b"Fassung von B!");
    fx
}

fn files_below(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_below(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn vault_task_compare_transfer_records_baseline() {
    let fx = Fx::new();
    fx.write(&fx.root_a, "/neu-a.txt", b"neu auf A");
    fx.write(&fx.root_a, "/Bericht.txt", b"Inhalt A, Fassung 2");
    fx.write(&fx.root_b, "/neu-b.txt", b"neu auf B");
    let view = fx.compare(&[]);
    for path in ["/neu-a.txt", "/Bericht.txt", "/neu-b.txt"] {
        let report = fx
            .resolve(item(&view, path), "transfer", false)
            .expect(path);
        assert_eq!(report.resolved, 1);
    }
    let (a, b) = (fx.tree(&fx.root_a), fx.tree(&fx.root_b));
    assert_eq!(a, b);
    assert_eq!(b["/neu-a.txt"], b"neu auf A");
    assert_eq!(a["/neu-b.txt"], b"neu auf B");
    // The entries are done: the next comparison is empty, and so is a run.
    assert!(fx.compare(&[]).items.is_empty());
    assert_converged(&fx);
}

#[test]
fn vault_task_compare_keep_a_b_with_versions() {
    for keep_a in [true, false] {
        let fx = conflicting();
        let view = fx.compare(&[CompareGroup::Conflict]);
        let entry = item(&view, "/konflikt.txt");
        let pair = {
            let state = cache().get(&fx.job.id).expect("kept comparison");
            let preview = state.preview.as_ref().expect("preview").lock().unwrap();
            preview
                .preview
                .state
                .as_ref()
                .expect("state")
                .pair_id
                .clone()
        };
        fx.resolve(entry, if keep_a { "keep-a" } else { "keep-b" }, false)
            .expect("kept");
        let (a, b) = (fx.tree(&fx.root_a), fx.tree(&fx.root_b));
        let winner: &[u8] = if keep_a {
            b"Fassung von A"
        } else {
            b"Fassung von B!"
        };
        assert_eq!(a["/konflikt.txt"], winner);
        assert_eq!(b["/konflikt.txt"], winner);
        // The loser went to the versions of the engine first.
        let mut versions = Vec::new();
        files_below(&crate::bisync::versions_dir(&pair), &mut versions);
        assert!(!versions.is_empty(), "the loser's version is kept");
        assert_converged(&fx);
    }
}

#[test]
fn vault_task_compare_keep_a_b_collective_action() {
    let fx = Fx::new();
    for name in ["eins", "zwei", "drei"] {
        fx.write(&fx.root_a, &format!("/{name}.txt"), b"von A");
        fx.write(&fx.root_b, &format!("/{name}.txt"), b"von B!");
    }
    fx.compare(&[CompareGroup::Conflict]);
    let report = fx
        .resolve_request(None, "keep-b", true, false)
        .expect("collective action");
    assert_eq!((report.resolved, report.failed.len()), (3, 0), "{report:?}");
    let a = fx.tree(&fx.root_a);
    assert_eq!(a["/eins.txt"], b"von B!");
    assert_eq!(a, fx.tree(&fx.root_b));
    // Only `keep-a` and `keep-b` are collective.
    assert!(fx.resolve_request(None, "transfer", true, false).is_err());
    assert_converged(&fx);
}

#[test]
fn vault_task_compare_keep_both_valid_cipher_name() {
    let fx = conflicting();
    let view = fx.compare(&[CompareGroup::Conflict]);
    let entry = item(&view, "/konflikt.txt");
    assert!(entry.actions.contains(&ItemAction::KeepBoth));
    fx.resolve(entry, "keep-both", false).expect("both kept");
    let a = fx.tree(&fx.root_a);
    assert_eq!(a, fx.tree(&fx.root_b), "both sides read alike");
    // The newer version (B) keeps the name, the other one a conflict name
    // that decrypts: not a cipher name nobody can read.
    assert_eq!(a["/konflikt.txt"], b"Fassung von B!");
    let copies: Vec<&String> = a
        .keys()
        .filter(|path| path.starts_with("/konflikt (Konflikt "))
        .collect();
    assert_eq!(copies.len(), 1, "{:?}", a.keys());
    assert!(copies[0].ends_with(" Tresor A).txt"), "{}", copies[0]);
    assert_eq!(a[copies[0].as_str()], b"Fassung von A");
    for root in [&fx.root_a, &fx.root_b] {
        let listing = crate::vfs::list_dir_tolerant(&fx.view(root), "/").expect("listing");
        assert!(listing.omitted.is_empty(), "{:?}", listing.omitted);
    }
    assert_converged(&fx);
    let test = "vault_task_compare_keep_both_valid_cipher_name";
    fx.keep(test, "a", &fx.root_a);
    fx.keep(test, "b", &fx.root_b);
}

#[test]
fn vault_task_compare_keep_both_long_name_c9s() {
    let fx = Fx::new();
    let name = format!("/{}.txt", "n".repeat(120));
    fx.write(&fx.root_a, &name, b"lang von A");
    tick();
    fx.write(&fx.root_b, &name, b"lang von B!");
    let before = stored(&fx.root_a);
    assert!(
        !before.keys().any(|rel| rel.contains(".c9s/")),
        "the original name is not shortened"
    );
    let view = fx.compare(&[CompareGroup::Conflict]);
    fx.resolve(item(&view, &name), "keep-both", false)
        .expect("both kept");
    // The conflict name is longer than the threshold: a `.c9s` entry.
    for root in [&fx.root_a, &fx.root_b] {
        let files = stored(root);
        assert!(files.keys().any(|rel| rel.contains(".c9s/name.c9s")));
        assert!(files.keys().any(|rel| rel.contains(".c9s/contents.c9r")));
    }
    let a = fx.tree(&fx.root_a);
    assert_eq!(a, fx.tree(&fx.root_b));
    assert_eq!(a.len(), fx.tree(&fx.root_b).len());
    assert!(a.keys().any(|path| path.contains("(Konflikt ")));
    assert_eq!(a[&name], b"lang von B!");
    assert_converged(&fx);
    let test = "vault_task_compare_keep_both_long_name_c9s";
    fx.keep(test, "a", &fx.root_a);
    fx.keep(test, "b", &fx.root_b);
}

#[test]
fn vault_task_compare_keep_both_baseline_recorded() {
    let fx = conflicting();
    let view = fx.compare(&[CompareGroup::Conflict]);
    fx.resolve(item(&view, "/konflikt.txt"), "keep-both", false)
        .expect("both kept");
    // The run that follows has nothing left to do.
    let run = fx.run();
    let stats = &run.outcome.stats;
    assert_eq!((stats.a_to_b, stats.b_to_a, stats.deleted), (0, 0, 0));
    assert!(run.outcome.conflicts.is_empty());
    assert!(run.outcome.errors.is_empty());
    assert!(fx.compare(&[]).items.is_empty(), "no entry is left");
}

#[test]
fn vault_task_compare_merge_inputs_contain_only_ciphertext() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let marker = format!("MARKE-{}-{nanos:x}", std::process::id());
    let fx = Fx::new();
    fx.write(
        &fx.root_a,
        "/text.txt",
        format!("A {marker}\ngemeinsam\n").as_bytes(),
    );
    fx.write(
        &fx.root_b,
        "/text.txt",
        format!("B {marker}\ngemeinsam\n").as_bytes(),
    );
    let view = fx.compare(&[CompareGroup::Conflict]);
    let entry = item(&view, "/text.txt");
    assert!(entry.actions.contains(&ItemAction::Merge));
    let (text_a, text_b) =
        super::compare_merge::compare_text_job(&fx.job.id, &entry.id).expect("texts");
    assert!(text_a.contains(&marker) && text_b.contains(&marker));
    let merged = format!("zusammen {marker}\ngemeinsam\n");
    super::compare_merge::merge_apply_item(&fx.job.id, &entry.id, &merged, &AtomicBool::new(false))
        .expect("merged");
    let (a, b) = (fx.tree(&fx.root_a), fx.tree(&fx.root_b));
    assert_eq!(a["/text.txt"], merged.as_bytes());
    assert_eq!(a, b);
    assert_converged(&fx);
    let test = "vault_task_compare_merge_inputs_contain_only_ciphertext";
    fx.keep(test, "a", &fx.root_a);
    fx.keep(test, "b", &fx.root_b);
    // No file of the engine (merge inputs, versions, state) and no file of
    // the vaults holds a cleartext line: ciphertext only.
    let mut files = Vec::new();
    files_below(&crate::support_dirs::app_data_dir(), &mut files);
    files_below(Path::new(&fx.root_a), &mut files);
    files_below(Path::new(&fx.root_b), &mut files);
    for file in files {
        let Ok(meta) = std::fs::metadata(&file) else {
            continue;
        };
        if meta.len() > 32 * 1024 * 1024 {
            continue;
        }
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };
        assert!(
            !bytes
                .windows(marker.len())
                .any(|window| window == marker.as_bytes()),
            "cleartext in {}",
            file.display()
        );
    }
}

#[test]
fn vault_task_compare_actions_check_the_entry() {
    let fx = Fx::new();
    fx.write(&fx.root_a, "/neu.txt", b"neu");
    let view = fx.compare(&[]);
    let entry = item(&view, "/neu.txt");
    // An action the entry does not offer is refused, an unknown one too.
    assert!(fx.resolve(entry, "keep-both", false).is_err());
    assert!(fx.resolve(entry, "kein-wort", false).is_err());
    assert!(fx.resolve(entry, "remove-node", true).is_err());
    fx.resolve(entry, "transfer", false).expect("transfer");
    // A done entry is gone: a second call is stale.
    let again = fx.resolve(entry, "transfer", false).expect_err("done");
    assert!(super::compare_cache::is_stale(&again), "{again}");
}
