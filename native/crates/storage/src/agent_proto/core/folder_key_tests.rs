//! Folder keys (E10): the key covers exactly the compared fields of a
//! folder's items in any order, and the key tree sends a known subtree as
//! one mark, every other folder as its run (parent first) with its key.
use super::*;
use std::collections::HashMap;

fn shape(name: &str, kind: EntryKind, size: u64, mtime_ms: i64) -> Shape<'_> {
    Shape {
        name,
        kind,
        size,
        mtime_ms,
    }
}

fn file(name: &str, size: u64, mtime_ms: i64) -> Vec<u8> {
    entry_part(name, EntryKind::File, size, mtime_ms, None, false)
}

#[test]
fn abgleich_task_folder_key_ignores_listing_order_and_follows_every_compared_field() {
    let a = run_digest(vec![file("a", 1, 10), file("b", 2, 20)]);
    let b = run_digest(vec![file("b", 2, 20), file("a", 1, 10)]);
    assert_eq!(a, b, "the listing order does not matter");
    for changed in [
        vec![file("a", 9, 10), file("b", 2, 20)],
        vec![file("a", 1, 11), file("b", 2, 20)],
        vec![file("A", 1, 10), file("b", 2, 20)],
        vec![
            entry_part("a", EntryKind::File, 1, 10, None, true),
            file("b", 2, 20),
        ],
        vec![
            entry_part("a", EntryKind::File, 1, 10, Some("00"), false),
            file("b", 2, 20),
        ],
        vec![
            entry_part("a", EntryKind::Link, 1, 10, None, false),
            file("b", 2, 20),
        ],
        vec![file("a", 1, 10), file("b", 2, 20), omission_part("c", 2)],
        vec![file("a", 1, 10), file("b", 2, 20), failure_part(2)],
    ] {
        assert_ne!(run_digest(changed), a);
    }
    // A folder's own size and time are not compared.
    assert_eq!(
        entry_part("d", EntryKind::Dir, 4096, 1, None, false),
        entry_part("d", EntryKind::Dir, 0, 2, None, false)
    );
    let child = folder_key(&a, Vec::new());
    let one = folder_key(&a, vec![("x".into(), child), ("y".into(), [7; 32])]);
    let other = folder_key(&a, vec![("y".into(), [7; 32]), ("x".into(), child)]);
    assert_eq!(one, other);
    assert_ne!(one, folder_key(&a, vec![("x".into(), child)]));
}

#[derive(Default)]
struct Recorded {
    out: Vec<String>,
}

impl KeyEmit<String> for Recorded {
    fn run(&mut self, rel: &str, items: Vec<String>) -> io::Result<()> {
        self.out.push(format!("run {rel}: {}", items.join(",")));
        Ok(())
    }

    fn key(
        &mut self,
        rel: &str,
        key: &FolderKey,
        same: bool,
        _: Option<KeyTally>,
    ) -> io::Result<()> {
        let kind = if same { "same" } else { "end" };
        self.out.push(format!("{kind} {rel} {:02x}", key[0]));
        Ok(())
    }
}

/// Root with file `f` and folders `a` (holding `a/g`) and `b` (empty).
fn walk(tree: &KeyTree<String>, out: &mut impl KeyEmit<String>) -> io::Result<()> {
    let mut root = RunKeys::new("");
    root.entry(
        shape("f", EntryKind::File, 1, 1),
        file("f", 1, 1),
        "f".into(),
        1,
    );
    for name in ["a", "b"] {
        let part = entry_part(name, EntryKind::Dir, 0, 0, None, false);
        root.entry(shape(name, EntryKind::Dir, 0, 0), part, name.into(), 1);
    }
    tree.finish_run(root, out)?;
    let mut a = RunKeys::new("a");
    a.entry(
        shape("g", EntryKind::File, 2, 2),
        file("g", 2, 2),
        "a/g".into(),
        1,
    );
    tree.finish_run(a, out)?;
    tree.finish_run(RunKeys::new("b"), out)
}

fn key_of(out: &Recorded, rel: &str) -> String {
    out.out
        .iter()
        .find(|line| line.starts_with(&format!("end {rel} ")))
        .unwrap_or_else(|| panic!("no key for {rel:?}: {:?}", out.out))
        .rsplit(' ')
        .next()
        .unwrap()
        .to_string()
}

#[test]
fn abgleich_task_key_tree_sends_every_folder_with_its_key_without_known_keys() {
    let tree = KeyTree::new("", HashSet::new());
    let mut out = Recorded::default();
    walk(&tree, &mut out).unwrap();
    assert!(tree.complete());
    let runs: Vec<&String> = out.out.iter().filter(|l| l.starts_with("run")).collect();
    assert_eq!(runs, ["run : f,a,b", "run a: a/g", "run b: "]);
    let end_root = out.out.iter().position(|l| l.starts_with("end  ")).unwrap();
    assert_eq!(end_root, out.out.len() - 1, "the root's key comes last");
    // Equal content, equal key: an empty folder and a fresh tree alike.
    let again = KeyTree::new("", HashSet::new());
    let mut second = Recorded::default();
    walk(&again, &mut second).unwrap();
    assert_eq!(key_of(&out, ""), key_of(&second, ""));
}

#[test]
fn abgleich_task_key_tree_answers_a_known_subtree_with_one_mark() {
    // First walk: learn the keys.
    let first = KeyTree::new("", HashSet::new());
    let mut learned = Recorded::default();
    walk(&first, &mut learned).unwrap();
    let mut keys = HashMap::new();
    let mut capture = Capture(&mut keys);
    let probe = KeyTree::new("", HashSet::new());
    walk(&probe, &mut capture).unwrap();
    // Everything known: only the root's mark goes out.
    let all: HashSet<KnownKey> = keys.values().map(known_prefix).collect();
    let tree = KeyTree::new("", all);
    let mut out = Recorded::default();
    walk(&tree, &mut out).unwrap();
    assert!(tree.complete());
    assert_eq!(out.out.len(), 1, "{:?}", out.out);
    assert!(out.out[0].starts_with("same  "), "{:?}", out.out);
    // Only `a` known: the root goes out with `b`, `a` is one mark after the
    // root's run.
    let only_a: HashSet<KnownKey> = [known_prefix(&keys["a"])].into();
    let tree = KeyTree::new("", only_a);
    let mut out = Recorded::default();
    walk(&tree, &mut out).unwrap();
    let position = |prefix: &str| out.out.iter().position(|l| l.starts_with(prefix));
    assert!(position("run a").is_none(), "{:?}", out.out);
    assert!(position("run : ").unwrap() < position("same a ").unwrap());
    assert!(position("run b").is_some() && position("end b ").is_some());
    assert_eq!(position("end  ").unwrap(), out.out.len() - 1);
}

struct Capture<'k>(&'k mut HashMap<String, FolderKey>);

impl KeyEmit<String> for Capture<'_> {
    fn run(&mut self, _: &str, _: Vec<String>) -> io::Result<()> {
        Ok(())
    }

    fn key(&mut self, rel: &str, key: &FolderKey, _: bool, _: Option<KeyTally>) -> io::Result<()> {
        self.0.insert(rel.to_string(), *key);
        Ok(())
    }
}

#[test]
fn abgleich_task_key_tree_refuses_a_folder_read_twice_or_an_unopened_child() {
    let tree = KeyTree::new("", HashSet::new());
    let mut out = Recorded::default();
    let mut root = RunKeys::new("");
    let part = entry_part("x", EntryKind::Dir, 0, 0, None, false);
    root.entry(shape("x", EntryKind::Dir, 0, 0), part, "x".into(), 1);
    // `x` could not be opened: it is no subfolder to wait for.
    root.omission("x", 2, "x!".into(), 1);
    tree.finish_run(root, &mut out).unwrap();
    assert!(tree.complete(), "{:?}", out.out);
    assert!(tree.finish_run(RunKeys::new(""), &mut out).is_err());
    let run: RunKeys<String> = RunKeys::new("p");
    assert!(run.name_of("q/r").is_err());
    assert_eq!(run.name_of("p/r").unwrap(), Some("r"));
    assert_eq!(run.name_of("p").unwrap(), None);
}

#[test]
fn abgleich_task_known_keys_are_whole_prefixes() {
    assert_eq!(parse_known(&[1; 32]).unwrap().len(), 1);
    assert!(parse_known(&[1; 17]).is_err());
}

struct Tallies(HashMap<String, Option<KeyTally>>);

impl KeyEmit<String> for Tallies {
    fn run(&mut self, _: &str, _: Vec<String>) -> io::Result<()> {
        Ok(())
    }

    fn key(
        &mut self,
        rel: &str,
        _: &FolderKey,
        _: bool,
        tally: Option<KeyTally>,
    ) -> io::Result<()> {
        self.0.insert(rel.to_string(), tally);
        Ok(())
    }
}

/// A folder of `files` (name, size, time) and `dirs`, as a run.
fn folder(rel: &str, files: &[(&str, u64, i64)], dirs: &[&str]) -> RunKeys<String> {
    let mut run = RunKeys::new(rel);
    for (name, size, time) in files {
        let part = file(name, *size, *time);
        run.entry(
            shape(name, EntryKind::File, *size, *time),
            part,
            (*name).into(),
            1,
        );
    }
    for name in dirs {
        let part = entry_part(name, EntryKind::Dir, 0, 0, None, false);
        run.entry(shape(name, EntryKind::Dir, 0, 0), part, (*name).into(), 1);
    }
    run
}

#[test]
fn abgleich_task_keys_only_tree_counts_each_subtree_for_the_caller() {
    let tree = KeyTree::with_mode("", HashSet::new(), Mode::KeysOnly);
    let mut out = Tallies(HashMap::new());
    tree.finish_run(folder("", &[("f", 5, 1)], &["a", "b"]), &mut out)
        .unwrap();
    tree.finish_run(folder("a", &[("g", 7, 2), ("h", 1, 0)], &["c"]), &mut out)
        .unwrap();
    tree.finish_run(folder("a/c", &[("i", 100, 3)], &[]), &mut out)
        .unwrap();
    let mut b = folder("b", &[], &[]);
    let part = entry_part("l", EntryKind::Link, 0, 0, None, false);
    b.entry(shape("l", EntryKind::Link, 0, 0), part, "l".into(), 1);
    b.entry(
        shape("x\\y", EntryKind::File, 3, 4),
        file("x\\y", 3, 4),
        "x".into(),
        1,
    );
    b.failure(2, "!".into(), 1);
    tree.finish_run(b, &mut out).unwrap();
    assert!(tree.complete());
    let tally = |rel: &str| out.0[rel].expect("tally");
    let c = tally("a/c");
    assert_eq!((c.files, c.bytes, c.height, c.clean()), (1, 100, 0, true));
    let a = tally("a");
    assert_eq!((a.files, a.bytes, a.untimed, a.height), (3, 108, 1, 1));
    assert!(
        !a.clean(),
        "a file of unknown time is not settled by its key"
    );
    let b = tally("b");
    // A link, a name with a backslash and the folder's own failure.
    assert_eq!((b.files, b.bytes, b.omitted), (1, 3, 3));
    let root = tally("");
    assert_eq!(
        (root.files, root.bytes, root.omitted, root.untimed),
        (5, 116, 3, 1)
    );
    assert_eq!(root.height, 2);
}

#[test]
fn abgleich_task_other_walk_modes_send_no_tally() {
    let tree = KeyTree::new("", HashSet::new());
    let mut out = Tallies(HashMap::new());
    tree.finish_run(folder("", &[("f", 5, 1)], &[]), &mut out)
        .unwrap();
    assert_eq!(out.0[""], None);
}
