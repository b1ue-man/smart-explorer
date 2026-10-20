//! The values of the vault comparison mode (vault plan C6, VLT-M8): the
//! groups of differences, the kinds of entries, the actions of an entry, one
//! entry with the state of both sides, and the view a surface shows. Pure
//! data and names: no I/O, no key and no platform code.
//!
//! The `key()` names are the wire names of the CLI, the daemon IPC and
//! the Android bridge (`conflict`, `changed_a`, … and `keep-a`, `keep-both`,
//! …); the `*_title` functions give the German texts the surfaces show.
use std::collections::BTreeMap;

/// The groups of C6.2, in the order surfaces list them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum CompareGroup {
    Conflict,
    ChangedA,
    ChangedB,
    NewA,
    NewB,
    DeletedA,
    DeletedB,
    Deferred,
    Structure,
}

impl CompareGroup {
    /// Every group, in listing order.
    pub(crate) const ALL: [CompareGroup; 9] = [
        Self::Conflict,
        Self::ChangedA,
        Self::ChangedB,
        Self::NewA,
        Self::NewB,
        Self::DeletedA,
        Self::DeletedB,
        Self::Deferred,
        Self::Structure,
    ];

    /// The groups a comparison lists unless the caller names others: all
    /// but `structure`, which only the check of a site (or a comparison
    /// asked for it) fills.
    pub(crate) const DEFAULT: [CompareGroup; 8] = [
        Self::Conflict,
        Self::ChangedA,
        Self::ChangedB,
        Self::NewA,
        Self::NewB,
        Self::DeletedA,
        Self::DeletedB,
        Self::Deferred,
    ];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Conflict => "conflict",
            Self::ChangedA => "changed_a",
            Self::ChangedB => "changed_b",
            Self::NewA => "new_a",
            Self::NewB => "new_b",
            Self::DeletedA => "deleted_a",
            Self::DeletedB => "deleted_b",
            Self::Deferred => "deferred",
            Self::Structure => "structure",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|group| group.key() == key)
    }

    /// How strongly a group claims an entry that has files of several
    /// groups (U26-4): the higher value wins.
    pub(crate) fn strength(self) -> u8 {
        match self {
            Self::Structure => 5,
            Self::Conflict => 4,
            Self::Deferred => 3,
            Self::DeletedA | Self::DeletedB => 2,
            Self::ChangedA | Self::ChangedB => 1,
            Self::NewA | Self::NewB => 0,
        }
    }
}

/// What an entry is to the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ItemKind {
    File,
    Folder,
    /// A technical entry without a cleartext name.
    Node,
    /// The key file, the configuration or a backup of them.
    Key,
}

impl ItemKind {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Folder => "folder",
            Self::Node => "node",
            Self::Key => "key",
        }
    }
}

/// What one side holds of an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SideState {
    pub(crate) exists: bool,
    /// The cleartext size of a file.
    pub(crate) size: Option<u64>,
    pub(crate) mtime_ms: Option<i64>,
    /// The label of the vault site on this side.
    pub(crate) location: String,
}

impl SideState {
    pub(crate) fn absent(location: &str) -> Self {
        Self {
            exists: false,
            size: None,
            mtime_ms: None,
            location: location.to_string(),
        }
    }
}

/// The actions of an entry (C6.4, C6.2). `KeepVersionA/B` share the wire
/// keys of `KeepA/B`: for a key file the same words read „Fassung A/B
/// behalten“.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ItemAction {
    KeepA,
    KeepB,
    KeepBoth,
    Merge,
    Transfer,
    ConfirmDelete,
    Restore,
    MergeFolders,
    KeepFoldersBoth,
    CanonicalName,
    RemoveDuplicate,
    SeparateFolder,
    RemoveNode,
    CreateContent,
    WriteDirId,
    AttachOrphan,
    RenameShortened,
    RewriteNameFile,
    RemoveStage,
    KeepVersionA,
    KeepVersionB,
}

impl ItemAction {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::KeepA | Self::KeepVersionA => "keep-a",
            Self::KeepB | Self::KeepVersionB => "keep-b",
            Self::KeepBoth => "keep-both",
            Self::Merge => "merge",
            Self::Transfer => "transfer",
            Self::ConfirmDelete => "confirm-delete",
            Self::Restore => "restore",
            Self::MergeFolders => "merge-folders",
            Self::KeepFoldersBoth => "keep-folders-both",
            Self::CanonicalName => "canonical-name",
            Self::RemoveDuplicate => "remove-duplicate",
            Self::SeparateFolder => "separate-folder",
            Self::RemoveNode => "remove-node",
            Self::CreateContent => "create-content",
            Self::WriteDirId => "write-dirid",
            Self::AttachOrphan => "attach-orphan",
            Self::RenameShortened => "rename-shortened",
            Self::RewriteNameFile => "rewrite-name-file",
            Self::RemoveStage => "remove-stage",
        }
    }

    /// The action a wire key names for an entry of `kind`: `keep-a` and
    /// `keep-b` are the version choice of a key file for `ItemKind::Key`.
    pub(crate) fn from_key(key: &str, kind: ItemKind) -> Option<Self> {
        let action = match key {
            "keep-a" if kind == ItemKind::Key => Self::KeepVersionA,
            "keep-b" if kind == ItemKind::Key => Self::KeepVersionB,
            "keep-a" => Self::KeepA,
            "keep-b" => Self::KeepB,
            "keep-both" => Self::KeepBoth,
            "merge" => Self::Merge,
            "transfer" => Self::Transfer,
            "confirm-delete" => Self::ConfirmDelete,
            "restore" => Self::Restore,
            "merge-folders" => Self::MergeFolders,
            "keep-folders-both" => Self::KeepFoldersBoth,
            "canonical-name" => Self::CanonicalName,
            "remove-duplicate" => Self::RemoveDuplicate,
            "separate-folder" => Self::SeparateFolder,
            "remove-node" => Self::RemoveNode,
            "create-content" => Self::CreateContent,
            "write-dirid" => Self::WriteDirId,
            "attach-orphan" => Self::AttachOrphan,
            "rename-shortened" => Self::RenameShortened,
            "rewrite-name-file" => Self::RewriteNameFile,
            "remove-stage" => Self::RemoveStage,
            _ => return None,
        };
        Some(action)
    }

    /// Every wire key, for help texts and completion.
    pub(crate) const KEYS: [&'static str; 19] = [
        "keep-a",
        "keep-b",
        "keep-both",
        "merge",
        "transfer",
        "confirm-delete",
        "restore",
        "merge-folders",
        "keep-folders-both",
        "canonical-name",
        "remove-duplicate",
        "separate-folder",
        "remove-node",
        "create-content",
        "write-dirid",
        "attach-orphan",
        "rename-shortened",
        "rewrite-name-file",
        "remove-stage",
    ];
}

/// One entry of the comparison: everything that belongs to one cleartext
/// path (several cipher files become one entry, U26-4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompareItem {
    pub(crate) id: String,
    pub(crate) group: CompareGroup,
    pub(crate) kind: ItemKind,
    /// The cleartext path (`/Ordner/Datei.txt`); empty for key files and
    /// technical entries.
    pub(crate) path: String,
    pub(crate) a: SideState,
    pub(crate) b: SideState,
    pub(crate) actions: Vec<ItemAction>,
    /// The CryptoFS name of a structure finding.
    pub(crate) finding: Option<String>,
    pub(crate) note: Option<String>,
}

/// What a surface shows of one comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompareView {
    pub(crate) job: String,
    pub(crate) site_a: String,
    pub(crate) site_b: String,
    /// The labels of the two sites (the `sides` of the JSON).
    pub(crate) label_a: String,
    pub(crate) label_b: String,
    pub(crate) revision: u64,
    pub(crate) items: Vec<CompareItem>,
    /// Entries per group key (every group, also with zero).
    pub(crate) counts: BTreeMap<String, usize>,
    /// Why the comparison could not plan (a safety stop of the engine).
    pub(crate) unavailable: Option<String>,
}

impl CompareView {
    /// A view of `items` between two sites (ID, label), with the counts of
    /// every group.
    pub(crate) fn new(
        job: &str,
        sites: [(&str, &str); 2],
        revision: u64,
        items: Vec<CompareItem>,
        unavailable: Option<String>,
    ) -> Self {
        let mut counts: BTreeMap<String, usize> = CompareGroup::ALL
            .iter()
            .map(|group| (group.key().to_string(), 0))
            .collect();
        for item in &items {
            if let Some(count) = counts.get_mut(item.group.key()) {
                *count += 1;
            }
        }
        Self {
            job: job.to_string(),
            site_a: sites[0].0.to_string(),
            label_a: sites[0].1.to_string(),
            site_b: sites[1].0.to_string(),
            label_b: sites[1].1.to_string(),
            revision,
            items,
            counts,
            unavailable,
        }
    }

    /// The entries of `groups`; an empty list selects every group.
    pub(crate) fn filtered(&self, groups: &[CompareGroup]) -> Vec<&CompareItem> {
        self.items
            .iter()
            .filter(|item| groups.is_empty() || groups.contains(&item.group))
            .collect()
    }
}

/// The German title of a group.
pub(crate) fn group_title(group: CompareGroup) -> &'static str {
    match group {
        CompareGroup::Conflict => "Konflikt",
        CompareGroup::ChangedA => "Geändert auf A",
        CompareGroup::ChangedB => "Geändert auf B",
        CompareGroup::NewA => "Neu auf A",
        CompareGroup::NewB => "Neu auf B",
        CompareGroup::DeletedA => "Auf A gelöscht",
        CompareGroup::DeletedB => "Auf B gelöscht",
        CompareGroup::Deferred => "Zurückgestellt",
        CompareGroup::Structure => "Struktur",
    }
}

/// The German title of an action.
pub(crate) fn action_title(action: ItemAction) -> &'static str {
    match action {
        ItemAction::KeepA | ItemAction::KeepVersionA => "Fassung A behalten",
        ItemAction::KeepB | ItemAction::KeepVersionB => "Fassung B behalten",
        ItemAction::KeepBoth => "Beide behalten",
        ItemAction::Merge => "Zusammenführen",
        ItemAction::Transfer => "Jetzt übertragen",
        ItemAction::ConfirmDelete => "Löschung übernehmen",
        ItemAction::Restore => "Wiederherstellen",
        ItemAction::MergeFolders => "Ordner zusammenführen",
        ItemAction::KeepFoldersBoth => "Beide Ordner behalten",
        ItemAction::CanonicalName => "Kanonisch benennen",
        ItemAction::RemoveDuplicate => "Doppelten Eintrag entfernen",
        ItemAction::SeparateFolder => "Ordner trennen",
        ItemAction::RemoveNode => "Knoten entfernen",
        ItemAction::CreateContent => "Leeren Inhalt anlegen",
        ItemAction::WriteDirId => "Verzeichnis-ID schreiben",
        ItemAction::AttachOrphan => "Unter LOST+FOUND einhängen",
        ItemAction::RenameShortened => "Ordner umbenennen",
        ItemAction::RewriteNameFile => "name.c9s neu schreiben",
        ItemAction::RemoveStage => "Stufe entfernen",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_task_compare_model_keys_round_trip() {
        for group in CompareGroup::ALL {
            assert_eq!(CompareGroup::from_key(group.key()), Some(group));
        }
        assert_eq!(CompareGroup::from_key("alle"), None);
        for key in ItemAction::KEYS {
            let file = ItemAction::from_key(key, ItemKind::File).expect(key);
            assert_eq!(file.key(), key);
        }
        // A key file reads keep-a/keep-b as the choice of a version.
        assert_eq!(
            ItemAction::from_key("keep-a", ItemKind::Key),
            Some(ItemAction::KeepVersionA)
        );
        assert_eq!(ItemAction::KeepVersionB.key(), "keep-b");
        assert_eq!(ItemAction::from_key("sync", ItemKind::File), None);
    }

    #[test]
    fn vault_task_compare_model_view_counts_every_group() {
        let side = SideState::absent("A");
        let item = |group| CompareItem {
            id: "x".into(),
            group,
            kind: ItemKind::File,
            path: "/a".into(),
            a: side.clone(),
            b: side.clone(),
            actions: Vec::new(),
            finding: None,
            note: None,
        };
        let view = CompareView::new(
            "job",
            [("a", "Tresor A"), ("b", "Tresor B")],
            1,
            vec![item(CompareGroup::Conflict), item(CompareGroup::NewB)],
            None,
        );
        assert_eq!(view.counts.len(), CompareGroup::ALL.len());
        assert_eq!(view.counts["conflict"], 1);
        assert_eq!(view.counts["deferred"], 0);
        assert_eq!(view.filtered(&[]).len(), 2);
        assert_eq!(view.filtered(&[CompareGroup::NewB]).len(), 1);
        assert!(CompareGroup::Conflict.strength() > CompareGroup::NewA.strength());
    }
}
