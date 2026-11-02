use super::*;

#[test]
fn sync_conflict_task_mobile_exposes_variants_and_disables_ambiguous_merge() {
    use crate::bisync::{DuplicateConflict, FileVariant};
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_string_lossy().replace('\\', "/");
    let variant = |id: &str, content: &[u8]| FileVariant {
        id: Some(id.into()), content_size: content.len() as u64,
        content_md5: format!("{:x}", md5::compute(content)),
        signature: Sig { size: content.len() as u64, mtime_ms: 123, hash: 1 },
    };
    let a = variant("a", b"A");
    let b = variant("b1", b"B");
    let context = PairContext {
        a: Arc::new(crate::vfs::LocalBackend::new(&root)), root_a: root.clone(),
        b: Arc::new(crate::vfs::LocalBackend::new(&root)), root_b: root.clone(), pair: root.clone(),
    };
    store_run(&root, context, vec![Conflict { rel: "Notebook/.obsidian/appearance.json".into(),
        a: Some(a.signature), b: Some(b.signature),
        duplicates: Some(DuplicateConflict { a: vec![a], b: vec![b, variant("b2", b"C")] }),
    }]);
    let result = conflicts(&json!({"id": root})).unwrap();
    let item = &result["items"][0];
    assert_eq!(item["a"]["needsVariantChoice"], false);
    assert_eq!(item["b"]["needsVariantChoice"], true);
    assert_eq!(item["b"]["variants"][1]["id"], "b2");
    assert_eq!(item["b"]["variants"][1]["checksum"], format!("{:x}", md5::compute(b"C")));
    assert_eq!(item["b"]["variants"][1]["size"], 1);
    assert_eq!(item["text"], false);
    forget(&root);
    assert_eq!(side(None, None, true)["exists"], false);
}
