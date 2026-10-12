//! What a listed or stat'ed entry is, as the terminal commands and the MCP
//! tools report it. A link is reported as a link (its own metadata), never as
//! the file or folder it points to.
use crate::vfs::VfsMeta;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EntryKind {
    File,
    Dir,
    /// Symbolic link, junction or another redirecting reparse point.
    Link,
    /// FIFO, socket or device: no data stream to read or copy.
    Special,
}

impl EntryKind {
    pub(super) fn of(meta: &VfsMeta) -> Self {
        if meta.is_symlink {
            Self::Link
        } else if meta.is_dir {
            Self::Dir
        } else if meta.special {
            Self::Special
        } else {
            Self::File
        }
    }

    /// One-letter column of `se ls` and `se search`.
    pub(super) fn marker(self) -> char {
        match self {
            Self::File => '-',
            Self::Dir => 'd',
            Self::Link => 'l',
            Self::Special => 's',
        }
    }

    /// Name in `se stat` and in MCP results.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Dir => "dir",
            Self::Link => "symlink",
            Self::Special => "special",
        }
    }

    /// Entries a tree copy skips and reports: links are never followed or
    /// recreated, special files carry no data.
    pub(super) fn is_omitted_in_trees(self) -> bool {
        matches!(self, Self::Link | Self::Special)
    }
}

#[cfg(test)]
mod tests {
    use super::EntryKind;
    use crate::vfs::VfsMeta;

    #[test]
    fn links_task_entry_kinds_prefer_the_link_over_its_target_type() {
        let mut meta = VfsMeta::default();
        assert_eq!(EntryKind::of(&meta), EntryKind::File);
        meta.special = true;
        assert_eq!(EntryKind::of(&meta), EntryKind::Special);
        meta.is_dir = true;
        meta.special = false;
        assert_eq!(EntryKind::of(&meta), EntryKind::Dir);
        meta.is_symlink = true;
        assert_eq!(EntryKind::of(&meta), EntryKind::Link);
        assert_eq!(EntryKind::Link.label(), "symlink");
        assert_eq!(EntryKind::Special.marker(), 's');
        assert!(EntryKind::Link.is_omitted_in_trees());
        assert!(!EntryKind::Dir.is_omitted_in_trees());
    }
}
