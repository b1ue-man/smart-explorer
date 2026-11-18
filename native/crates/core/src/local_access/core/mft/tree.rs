//! A plain size tree built from a walk.
use super::index::MftIndex;
use super::walk::{MftEntry, MftKind, MftSection, MftTotals, MftVisitor};
use super::{FileRef, MftDiagnostics, ROOT_RECORD};
use std::io;
use std::sync::atomic::AtomicBool;

/// Name of the separate node that holds the NTFS system files.
pub const SYSTEM_FILES_NAME: &str = "NTFS-Systemdateien";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MftNode {
    pub name: String,
    pub name_exact: bool,
    pub kind: MftKind,
    pub record: u64,
    /// Logical bytes (directories: sum of their subtree).
    pub size: u64,
    /// Occupied bytes, hard links counted once (directories: subtree sum).
    pub allocated: u64,
    pub repeated_link: bool,
    pub attributes: u32,
    pub children: Vec<MftNode>,
}

impl MftNode {
    fn from_entry(entry: &MftEntry<'_>) -> Self {
        Self {
            name: entry.name.to_string(),
            name_exact: entry.name_exact,
            kind: entry.kind,
            record: entry.record,
            size: entry.size,
            allocated: entry.allocated,
            repeated_link: entry.repeated_link,
            attributes: entry.attributes,
            children: Vec::new(),
        }
    }
}

/// The result of one MFT scan.
#[derive(Clone, Debug)]
pub struct MftScan {
    pub root: MftNode,
    /// `$MFT`, `$LogFile`, `$Extend`, ... as one directory node named
    /// `SYSTEM_FILES_NAME`; only for a scan of the volume root.
    pub system_files: Option<MftNode>,
    /// Defects of the MFT and of the walk; `partial` means sizes are lower
    /// bounds.
    pub diagnostics: MftDiagnostics,
}

struct TreeBuilder<'k> {
    keep: &'k mut dyn FnMut(&MftEntry<'_>) -> bool,
    stack: Vec<MftNode>,
    done: Option<MftNode>,
}

impl MftVisitor for TreeBuilder<'_> {
    fn keep(&mut self, entry: &MftEntry<'_>) -> bool {
        (self.keep)(entry)
    }
    fn enter_directory(&mut self, entry: &MftEntry<'_>) {
        self.stack.push(MftNode::from_entry(entry));
    }
    fn leaf(&mut self, entry: &MftEntry<'_>) {
        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(MftNode::from_entry(entry));
        }
    }
    fn leave_directory(&mut self, totals: &MftTotals) {
        let Some(mut node) = self.stack.pop() else {
            return;
        };
        node.size = totals.size;
        node.allocated = totals.allocated;
        match self.stack.last_mut() {
            Some(parent) => parent.children.push(node),
            None => self.done = Some(node),
        }
    }
}

impl MftIndex {
    fn tree(
        &self,
        root: FileRef,
        name: &str,
        section: MftSection,
        cancel: &AtomicBool,
        keep: &mut dyn FnMut(&MftEntry<'_>) -> bool,
        diagnostics: &mut MftDiagnostics,
    ) -> io::Result<MftNode> {
        let mut builder = TreeBuilder {
            keep,
            stack: Vec::new(),
            done: None,
        };
        let (_, walked) = self.walk(root, name, section, cancel, &mut builder)?;
        diagnostics.absorb(walked);
        builder
            .done
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "MFT-Baum unvollständig"))
    }

    /// The size tree of directory `root`, named `root_name`. `keep` decides
    /// which entries (and subtrees) enter it. With `system_files` and the
    /// volume root, the NTFS system files form `MftScan::system_files`.
    pub fn build_tree(
        &self,
        root: FileRef,
        root_name: &str,
        system_files: bool,
        cancel: &AtomicBool,
        keep: &mut dyn FnMut(&MftEntry<'_>) -> bool,
    ) -> io::Result<MftScan> {
        let mut diagnostics = self.diagnostics.clone();
        let tree = self.tree(
            root,
            root_name,
            MftSection::Content,
            cancel,
            keep,
            &mut diagnostics,
        )?;
        let system_files = if system_files && root.record == ROOT_RECORD {
            Some(self.tree(
                root,
                SYSTEM_FILES_NAME,
                MftSection::SystemFiles,
                cancel,
                keep,
                &mut diagnostics,
            )?)
        } else {
            None
        };
        Ok(MftScan {
            root: tree,
            system_files,
            diagnostics,
        })
    }
}
