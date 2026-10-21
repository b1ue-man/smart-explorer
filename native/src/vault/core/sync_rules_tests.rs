//! The rule stage and the apply phases of vault sync jobs on plans built by
//! hand (vault plan C5.4): no storage, no key.
use super::*;
use crate::bisync::plan_rules::phase_plan;
use crate::bisync::Sig;

const ROOT: &str = "d/AB/CDEFGHIJKLMNOPQRSTUVWXYZ234567";
const SUB: &str = "d/QR/STUVWXYZ234567ABCDEFGHIJKLMNOP";

fn sig(size: u64) -> Sig {
    Sig {
        size,
        mtime_ms: 1_000,
        hash: 0,
    }
}

fn plan_of(actions: Vec<Action>) -> PairPlan {
    PairPlan {
        actions,
        ..PairPlan::default()
    }
}

#[test]
fn cipher_paths_classify_by_the_format_8_layout() {
    assert_eq!(classify("masterkey.cryptomator"), CipherEntry::Metadata);
    assert_eq!(
        classify("vault.cryptomator.1A2B3C4D.bkup"),
        CipherEntry::Metadata
    );
    assert_eq!(classify("notes.txt"), CipherEntry::Other);
    let node = format!("{ROOT}/aGVsbG8=.c9r/dir.c9r");
    assert_eq!(classify(&node), CipherEntry::Node { content: ROOT });
    let long = format!("{ROOT}/aGVsbG8.c9s/symlink.c9r");
    assert_eq!(classify(&long), CipherEntry::Node { content: ROOT });
    for content in ["dirid.c9r", "x.c9r", "y.c9s/contents.c9r", "y.c9s/name.c9s"] {
        let rel = format!("{ROOT}/{content}");
        assert_eq!(
            classify(&rel),
            CipherEntry::Content { content: ROOT },
            "{rel}"
        );
    }
    assert!(is_dir_id_backup(&format!("{ROOT}/dirid.c9r")));
    assert!(!is_dir_id_backup(&format!("{ROOT}/x.c9r")));
    // Phases (R4): node files after content, deletions below d/ after node
    // deletions.
    assert_eq!(phase_of(&Action::CopyAtoB(node.clone())), 2);
    assert_eq!(phase_of(&Action::DeleteB(node)), 1);
    assert_eq!(phase_of(&Action::CopyBtoA(format!("{ROOT}/dirid.c9r"))), 1);
    assert_eq!(phase_of(&Action::DeleteA(format!("{ROOT}/x.c9r"))), 2);
    assert_eq!(
        phase_of(&Action::CopyAtoB("masterkey.cryptomator".into())),
        1
    );
}

#[test]
fn vault_task_sync_keepboth_actions_become_conflicts() {
    let rules = VaultRules::new(None);
    let file = format!("{ROOT}/eins.c9r");
    let other = format!("{ROOT}/zwei.c9r");
    let mut plan = plan_of(vec![
        Action::KeepBothAtoB(file.clone()),
        Action::KeepBothBtoA(other.clone()),
        Action::CopyAtoB(format!("{ROOT}/drei.c9r")),
    ]);
    rules.adjust(&mut plan);
    // R10: no keep-both action survives; each became a conflict.
    assert!(!plan
        .actions
        .iter()
        .any(|action| matches!(action, Action::KeepBothAtoB(_) | Action::KeepBothBtoA(_))));
    let mut conflicts: Vec<&str> = plan
        .conflicts
        .iter()
        .map(|item| item.rel.as_str())
        .collect();
    conflicts.sort_unstable();
    assert_eq!(conflicts, [file.as_str(), other.as_str()]);
    assert_eq!(plan.actions, [Action::CopyAtoB(format!("{ROOT}/drei.c9r"))]);
    // R1: the key file and the configuration are never deleted.
    let mut plan = plan_of(vec![
        Action::DeleteB("masterkey.cryptomator".into()),
        Action::DeleteA("vault.cryptomator".into()),
        Action::DeleteB("vault.cryptomator.0A0B0C0D.bkup".into()),
    ]);
    rules.adjust(&mut plan);
    assert_eq!(
        plan.actions,
        [Action::DeleteB("vault.cryptomator.0A0B0C0D.bkup".into())]
    );
    assert_eq!(plan.conflicts.len(), 2);
    // R9: stage names of se never travel.
    let mut plan = plan_of(vec![Action::CopyAtoB(format!(
        "{ROOT}/.x.c9r.smart-explorer-0123456789abcdef.part"
    ))]);
    rules.adjust(&mut plan);
    assert!(plan.actions.is_empty());
    assert_eq!(rules.adjusted(), 3);
}

#[test]
fn folder_deleted_on_one_side_and_changed_on_the_other_waits_for_comparison() {
    let rules = VaultRules::new(None);
    let node_sub = format!("{ROOT}/c3Vi.c9r/dir.c9r");
    let other_node = format!("{ROOT}/YW5kZXJz.c9r/dir.c9r");
    let mut plan = plan_of(vec![
        // A deleted the folder SUB: its entries and node go on B …
        Action::DeleteB(format!("{SUB}/dirid.c9r")),
        Action::DeleteB(format!("{SUB}/alt.c9r")),
        Action::DeleteB(node_sub.clone()),
        // … while B added a file in it, and another folder's node goes too.
        Action::CopyBtoA(format!("{SUB}/neu.c9r")),
        Action::DeleteB(other_node.clone()),
        Action::CopyAtoB(format!("{ROOT}/frei.c9r")),
    ]);
    plan.conflicts.push(Conflict {
        rel: format!("{SUB}/geaendert.c9r"),
        a: None,
        b: Some(sig(9)),
        duplicates: None,
    });
    plan.dirs.push(DirAction::Create {
        side: PairSide::A,
        rel: SUB.to_string(),
    });
    rules.adjust(&mut plan);
    assert!(plan.dirs.is_empty(), "no empty content folder is recreated");
    let phases = phase_plan(&rules, &plan.actions).expect("rule phases");
    let deferred: Vec<&str> = phases
        .deferred
        .iter()
        .map(|&index| action_rel(&plan.actions[index]))
        .collect();
    for rel in [
        format!("{SUB}/dirid.c9r"),
        format!("{SUB}/alt.c9r"),
        format!("{SUB}/neu.c9r"),
        node_sub.clone(),
        other_node.clone(),
    ] {
        assert!(deferred.contains(&rel.as_str()), "{rel} not deferred");
    }
    assert_eq!(deferred.len(), 5);
    assert_eq!(
        rules.deferral_reason(&Action::DeleteB(other_node)),
        DEFERRED_NODE_DELETION
    );
    assert_eq!(
        rules.deferral_reason(&Action::CopyBtoA(format!("{SUB}/neu.c9r"))),
        DEFERRED_FOLDER_CONFLICT
    );
    // The free copy runs; the conflict stays a conflict.
    assert_eq!(phases.phases.concat().len(), 1);
    assert_eq!(plan.conflicts.len(), 1);
}

#[test]
fn incomplete_phase_one_holds_node_creations_and_deletions_of_its_folder() {
    let rules = VaultRules::new(None);
    let failed = Action::CopyAtoB(format!("{SUB}/dirid.c9r"));
    let coming = [
        Action::CopyAtoB(format!("{ROOT}/bmV1.c9r/dir.c9r")),
        Action::DeleteB(format!("{SUB}/weg.c9r")),
        Action::DeleteB(format!("{ROOT}/auch-weg.c9r")),
    ];
    let coming: Vec<&Action> = coming.iter().collect();
    assert_eq!(rules.held_back(&[&failed], &coming), [0, 1]);
    // An unfinished node deletion holds every deletion below d/.
    let node_deletion = Action::DeleteB(format!("{ROOT}/YWx0.c9r/dir.c9r"));
    assert_eq!(rules.held_back(&[&node_deletion], &coming), [0, 1, 2]);
    assert!(rules.held_back(&[], &coming).is_empty());
}

#[test]
fn a_copy_being_seeded_only_receives() {
    let rules = VaultRules::new(Some(PairSide::B));
    let mut plan = plan_of(vec![
        Action::DeleteA(format!("{ROOT}/verloren.c9r")),
        Action::DeleteB(format!("{ROOT}/entfernt.c9r")),
        Action::CopyAtoB(format!("{ROOT}/neu.c9r")),
    ]);
    plan.conflicts.push(Conflict {
        rel: format!("{ROOT}/fehlt.c9r"),
        a: Some(sig(4)),
        b: None,
        duplicates: None,
    });
    plan.conflicts.push(Conflict {
        rel: format!("{ROOT}/beide.c9r"),
        a: Some(sig(4)),
        b: Some(sig(5)),
        duplicates: None,
    });
    plan.dirs.push(DirAction::Remove {
        side: PairSide::A,
        rel: ROOT.to_string(),
    });
    rules.adjust(&mut plan);
    assert!(!plan
        .actions
        .iter()
        .any(|action| matches!(action, Action::DeleteA(_))));
    assert!(plan
        .actions
        .contains(&Action::CopyAtoB(format!("{ROOT}/verloren.c9r"))));
    assert!(plan
        .actions
        .contains(&Action::CopyAtoB(format!("{ROOT}/fehlt.c9r"))));
    assert!(plan
        .actions
        .contains(&Action::DeleteB(format!("{ROOT}/entfernt.c9r"))));
    assert_eq!(
        plan.conflicts.len(),
        1,
        "a real divergence stays a conflict"
    );
    assert!(plan.dirs.is_empty());
}

#[test]
fn node_folders_are_never_created_empty_and_emptied_ones_are_removed() {
    let rules = VaultRules::new(None);
    let gone = format!("{ROOT}/YWx0.c9r");
    let long = format!("{ROOT}/bGFuZw.c9s");
    // B's folder `gone` and B's long-named file lost their files on A.
    let mut plan = plan_of(vec![
        Action::DeleteB(format!("{gone}/dir.c9r")),
        Action::DeleteB(format!("{long}/contents.c9r")),
        Action::DeleteB(format!("{long}/name.c9s")),
        Action::DeleteB(format!("{ROOT}/frei.c9r")),
    ]);
    let create = |rel: &str| DirAction::Create {
        side: PairSide::A,
        rel: rel.to_string(),
    };
    plan.dirs.push(create(&gone));
    plan.dirs.push(create(ROOT));
    plan.dirs.push(DirAction::Remove {
        side: PairSide::B,
        rel: ROOT.to_string(),
    });
    rules.adjust(&mut plan);
    assert!(
        !plan.dirs.contains(&create(&gone)),
        "no empty node folder on the side that deleted it"
    );
    assert!(
        plan.dirs.contains(&create(ROOT)),
        "content folders keep the engine's folder plan"
    );
    let remove = |side, rel: &str| DirAction::Remove {
        side,
        rel: rel.to_string(),
    };
    let removals: Vec<DirAction> = plan
        .dirs
        .iter()
        .filter(|dir| matches!(dir, DirAction::Remove { .. }))
        .cloned()
        .collect();
    assert_eq!(
        removals,
        [
            remove(PairSide::B, &gone),
            remove(PairSide::B, &long),
            remove(PairSide::B, ROOT)
        ],
        "each emptied node folder once, before the folder around them"
    );
}

#[test]
fn node_folders_of_a_held_back_deletion_are_not_removed() {
    let rules = VaultRules::new(None);
    let node = format!("{ROOT}/YWx0.c9r");
    // R2: A deleted the folder SUB, B changed it; the node deletion waits.
    let mut plan = plan_of(vec![
        Action::DeleteB(format!("{SUB}/dirid.c9r")),
        Action::CopyBtoA(format!("{SUB}/neu.c9r")),
        Action::DeleteB(format!("{node}/dir.c9r")),
    ]);
    rules.adjust(&mut plan);
    assert!(plan.dirs.is_empty(), "{:?}", plan.dirs);
}

#[test]
fn node_folder_that_receives_a_new_type_file_stays() {
    let rules = VaultRules::new(None);
    let node = format!("{ROOT}/YWx0.c9r");
    // A replaced the link with a folder of the same name: B's node folder
    // loses `symlink.c9r` and receives `dir.c9r` in the same run.
    let mut plan = plan_of(vec![
        Action::DeleteB(format!("{node}/symlink.c9r")),
        Action::CopyAtoB(format!("{node}/dir.c9r")),
    ]);
    rules.adjust(&mut plan);
    assert!(plan.dirs.is_empty(), "{:?}", plan.dirs);
}
