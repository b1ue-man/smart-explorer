use super::*;

fn meta(name: &str) -> VfsMeta {
    VfsMeta {
        name: name.into(),
        is_dir: false,
        is_symlink: false,
        special: false,
        size: 7,
        mtime_ms: 0,
        btime_ms: 0,
        hidden: false,
        system: false,
        id: None,
        content_md5: None,
    }
}

#[test]
fn windows_remote_task_peer_names_are_stable_exact_and_reversible() {
    let raw = vec![
        meta("Docs"),
        meta("docs"),
        meta("Straße.txt"),
        meta("STRASSE.txt"),
    ];
    let projected = project_peer_listing(raw.clone()).unwrap();
    assert_eq!(projected.len(), raw.len());
    assert_eq!(projected[2].name, "Straße.txt");
    assert_eq!(projected[3].name, "STRASSE.txt");
    let mut reversed = raw.clone();
    reversed.reverse();
    let mut reversed = project_peer_listing(reversed).unwrap();
    reversed.reverse();
    for (index, entry) in projected.iter().enumerate() {
        assert_eq!(entry.name, reversed[index].name);
        let (resolved, _) = resolve_peer_child(raw.clone(), &entry.name)
            .unwrap()
            .unwrap();
        assert_eq!(resolved.name, raw[index].name);
    }
    assert!(resolve_peer_child(raw.clone(), "DOCS").is_err());
    assert!(project_peer_listing(vec![meta("same"), meta("same")]).is_err());
    let alias = &projected[0].name;
    let lookalike = meta(alias);
    let with_lookalike = vec![raw[0].clone(), lookalike.clone()];
    let projected_again = project_peer_listing(with_lookalike.clone()).unwrap();
    assert_ne!(projected_again[1].name, *alias);
    assert_eq!(
        resolve_peer_child(with_lookalike, alias)
            .unwrap()
            .unwrap()
            .0
            .name,
        "Docs"
    );
    assert!(resolve_peer_child(vec![lookalike], alias)
        .unwrap()
        .is_none());
}

#[test]
fn windows_remote_task_peer_aliases_preserve_extensions_and_staging() {
    let long = format!("{}.txt", "😀".repeat(124));
    let alias = peer_alias(&long);
    assert!(alias.encode_utf16().count() <= 255);
    assert!(alias.ends_with(".txt"));
    assert!(is_peer_alias(&alias));
    let raw = vec![meta("Note.txt"), meta("note.txt")];
    let projected = project_peer_listing(raw.clone()).unwrap();
    for entry in projected {
        assert!(entry.name.ends_with(".txt"));
        assert!(is_peer_alias(&entry.name));
        assert!(resolve_peer_child(raw.clone(), &entry.name.to_uppercase())
            .unwrap()
            .is_some());
        for suffix in [
            ".se-mount-1234567890abcdef",
            ".se-mount-delete-1234567890abcdef",
        ] {
            assert!(!is_peer_alias(&format!("{}{suffix}", entry.name)));
            assert!(!is_peer_alias(&format!("{}{suffix}", peer_alias("folder"))));
        }
    }
    let hidden_stage = format!("{}.se-mount-1234567890abcdef", peer_alias("folder"));
    assert_eq!(
        project_peer_listing(vec![meta(&hidden_stage)]).unwrap()[0].name,
        hidden_stage
    );
}
