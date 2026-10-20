//! The findings of „Tresor prüfen“ (vault plan C6.2, C6.6, U26-7): the
//! 17 findings of CryptoFS 2.11.0 (three of them healthy) and four of se,
//! with the actions a finding offers and what an action needs before it
//! runs. Pure: the walk that finds them and the fixes are in
//! `os/shared/{dir_map,health,health_fixes}.rs`.
use super::compare_model::ItemAction;

/// What a check found. `name()` is the wire name of the `finding` field:
/// the English name CryptoFS gives the finding, or the name of se's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum FindingKind {
    // The three healthy ones of CryptoFS: never listed.
    HealthyDir,
    ValidShortenedFile,
    KnownType,
    // The 14 findings of CryptoFS 2.11.0.
    DirIdCollision,
    EmptyDirFile,
    ObeseDirFile,
    LooseDirFile,
    MissingContentDir,
    MissingDirIdBackup,
    OrphanContentDir,
    LongShortNamesMismatch,
    MissingLongName,
    NotDecodableLongName,
    ObeseNameFile,
    TrailingBytesInNameFile,
    UnknownType,
    AmbiguousType,
    // The four of se.
    UndecryptableName,
    ForeignConflictCopy,
    StageLeftover,
    KeyFileDiffers,
}

impl FindingKind {
    /// Every finding.
    pub(crate) const ALL: [FindingKind; 21] = [
        Self::HealthyDir,
        Self::ValidShortenedFile,
        Self::KnownType,
        Self::DirIdCollision,
        Self::EmptyDirFile,
        Self::ObeseDirFile,
        Self::LooseDirFile,
        Self::MissingContentDir,
        Self::MissingDirIdBackup,
        Self::OrphanContentDir,
        Self::LongShortNamesMismatch,
        Self::MissingLongName,
        Self::NotDecodableLongName,
        Self::ObeseNameFile,
        Self::TrailingBytesInNameFile,
        Self::UnknownType,
        Self::AmbiguousType,
        Self::UndecryptableName,
        Self::ForeignConflictCopy,
        Self::StageLeftover,
        Self::KeyFileDiffers,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::HealthyDir => "HealthyDir",
            Self::ValidShortenedFile => "ValidShortenedFile",
            Self::KnownType => "KnownType",
            Self::DirIdCollision => "DirIdCollision",
            Self::EmptyDirFile => "EmptyDirFile",
            Self::ObeseDirFile => "ObeseDirFile",
            Self::LooseDirFile => "LooseDirFile",
            Self::MissingContentDir => "MissingContentDir",
            Self::MissingDirIdBackup => "MissingDirIdBackup",
            Self::OrphanContentDir => "OrphanContentDir",
            Self::LongShortNamesMismatch => "LongShortNamesMismatch",
            Self::MissingLongName => "MissingLongName",
            Self::NotDecodableLongName => "NotDecodableLongName",
            Self::ObeseNameFile => "ObeseNameFile",
            Self::TrailingBytesInNameFile => "TrailingBytesInNameFile",
            Self::UnknownType => "UnknownType",
            Self::AmbiguousType => "AmbiguousType",
            Self::UndecryptableName => "UndecryptableName",
            Self::ForeignConflictCopy => "ForeignConflictCopy",
            Self::StageLeftover => "StageLeftover",
            Self::KeyFileDiffers => "KeyFileDiffers",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// Healthy findings are no findings of the check.
    pub(crate) fn is_healthy(self) -> bool {
        matches!(
            self,
            Self::HealthyDir | Self::ValidShortenedFile | Self::KnownType
        )
    }

    /// The actions a finding offers (C6.2); `same_parent_and_name` tells a
    /// `DirIdCollision` whose two nodes lie in one folder under one name.
    pub(crate) fn actions(self, same_parent_and_name: bool) -> Vec<ItemAction> {
        match self {
            Self::DirIdCollision if same_parent_and_name => vec![ItemAction::RemoveDuplicate],
            Self::DirIdCollision => vec![ItemAction::SeparateFolder],
            Self::EmptyDirFile | Self::ObeseDirFile | Self::LooseDirFile | Self::UnknownType => {
                vec![ItemAction::RemoveNode]
            }
            Self::MissingContentDir => vec![ItemAction::CreateContent],
            Self::MissingDirIdBackup => vec![ItemAction::WriteDirId],
            Self::OrphanContentDir => vec![ItemAction::AttachOrphan],
            Self::LongShortNamesMismatch => vec![ItemAction::RenameShortened],
            Self::TrailingBytesInNameFile => vec![ItemAction::RewriteNameFile],
            Self::ForeignConflictCopy => vec![ItemAction::CanonicalName],
            Self::StageLeftover => vec![ItemAction::RemoveStage],
            Self::KeyFileDiffers => vec![ItemAction::KeepVersionA, ItemAction::KeepVersionB],
            Self::HealthyDir
            | Self::ValidShortenedFile
            | Self::KnownType
            | Self::MissingLongName
            | Self::NotDecodableLongName
            | Self::ObeseNameFile
            | Self::AmbiguousType
            | Self::UndecryptableName => Vec::new(),
        }
    }

    /// Short German description of the finding for surfaces.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::HealthyDir | Self::ValidShortenedFile | Self::KnownType => "in Ordnung",
            Self::DirIdCollision => "doppelte Verzeichnis-ID",
            Self::EmptyDirFile | Self::ObeseDirFile => "Ordnerknoten beschädigt",
            Self::LooseDirFile => "Knotendatei ohne Ordner",
            Self::MissingContentDir => "Ordner ohne Inhalt",
            Self::MissingDirIdBackup => "Sicherung der Verzeichnis-ID fehlt",
            Self::OrphanContentDir => "verwaistes Inhaltsverzeichnis",
            Self::LongShortNamesMismatch => "Kurzname passt nicht zum langen Namen",
            Self::MissingLongName | Self::NotDecodableLongName | Self::ObeseNameFile => {
                "langer Name verloren oder beschädigt"
            }
            Self::TrailingBytesInNameFile => "überzählige Bytes in name.c9s",
            Self::UnknownType => "Knoten ohne Typdatei",
            Self::AmbiguousType => "mehrdeutiger Knoten",
            Self::UndecryptableName => "Name nicht entschlüsselbar",
            Self::ForeignConflictCopy => "Konfliktkopie eines fremden Programms",
            Self::StageLeftover => "liegengebliebene Stufe von se",
            Self::KeyFileDiffers => "Schlüsseldatei auf den Seiten verschieden",
        }
    }
}

/// One finding of a check: what, where (relative to the vault root) and a
/// detail the fix needs (never a cleartext name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HealthFinding {
    pub(crate) kind: FindingKind,
    pub(crate) rel: String,
    pub(crate) detail: Option<String>,
}

impl HealthFinding {
    pub(crate) fn new(kind: FindingKind, rel: &str, detail: Option<String>) -> Self {
        Self {
            kind,
            rel: rel.to_string(),
            detail,
        }
    }
}

/// Whether an action removes cipher files (U26-7: a copy of them goes to
/// `vault-removed/` first and the surface asks for confirmation).
pub(crate) fn removes_cipher_files(action: ItemAction) -> bool {
    matches!(
        action,
        ItemAction::RemoveNode
            | ItemAction::RemoveDuplicate
            | ItemAction::RemoveStage
            | ItemAction::SeparateFolder
            | ItemAction::MergeFolders
    )
}

/// Whether an action must wait for the pair lock of the vault jobs that use
/// the site: it may look like the half-finished state of a running sync.
pub(crate) fn needs_free_pair_lock(kind: FindingKind) -> bool {
    matches!(
        kind,
        FindingKind::MissingContentDir | FindingKind::UnknownType
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_task_health_model_names_and_actions() {
        assert_eq!(FindingKind::ALL.len(), 21);
        for kind in FindingKind::ALL {
            assert_eq!(FindingKind::from_name(kind.name()), Some(kind));
            assert!(!kind.describe().is_empty());
            if kind.is_healthy() {
                assert!(kind.actions(false).is_empty());
            }
        }
        assert_eq!(
            FindingKind::DirIdCollision.actions(true),
            vec![ItemAction::RemoveDuplicate]
        );
        assert_eq!(
            FindingKind::DirIdCollision.actions(false),
            vec![ItemAction::SeparateFolder]
        );
        // CryptoFS offers no repair for these; se only shows them.
        for kind in [
            FindingKind::MissingLongName,
            FindingKind::NotDecodableLongName,
            FindingKind::ObeseNameFile,
            FindingKind::AmbiguousType,
            FindingKind::UndecryptableName,
        ] {
            assert!(kind.actions(false).is_empty(), "{}", kind.name());
        }
        assert!(removes_cipher_files(ItemAction::RemoveNode));
        assert!(!removes_cipher_files(ItemAction::WriteDirId));
        assert!(needs_free_pair_lock(FindingKind::UnknownType));
        assert!(!needs_free_pair_lock(FindingKind::StageLeftover));
    }
}
