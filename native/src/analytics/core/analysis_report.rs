use crate::analytics::{
    tree_transfer::TreeShape, ScanIssue, ScanOutcome, ScanSnapshot, ScanStatus, SizeNode,
};
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct AnalysisReport {
    pub(crate) status: ScanStatus,
    pub(crate) issues: Vec<ScanIssue>,
    pub(crate) suppressed_issues: u64,
    pub(crate) permission_denied: u64,
    pub(crate) notes: Vec<String>,
    pub(crate) aggregated_files: u64,
    pub(crate) progress: ScanSnapshot,
    pub(crate) shape: TreeShape,
}

impl AnalysisReport {
    pub(crate) fn take(
        outcome: &mut ScanOutcome,
        progress: ScanSnapshot,
        shape: TreeShape,
    ) -> Self {
        Self {
            status: outcome.status,
            issues: std::mem::take(&mut outcome.issues),
            suppressed_issues: outcome.suppressed_issues,
            permission_denied: outcome.permission_denied,
            notes: std::mem::take(&mut outcome.notes),
            aggregated_files: outcome.aggregated_files,
            progress,
            shape,
        }
    }

    pub(crate) fn validate(&self) -> io::Result<()> {
        self.shape.validate()?;
        let has_tree = matches!(self.status, ScanStatus::Complete | ScanStatus::Partial);
        let diagnostic_bytes: usize = self
            .issues
            .iter()
            .map(|issue| issue.path.len() + issue.detail.len())
            .sum::<usize>()
            + self.notes.iter().map(String::len).sum::<usize>();
        if has_tree != (self.shape.nodes > 0)
            || self.issues.len() > 64
            || self.notes.len() > 16
            || diagnostic_bytes > 512 * 1024
            || (self.status == ScanStatus::Complete
                && (!self.issues.is_empty() || self.suppressed_issues != 0))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Widersprüchlicher Analyse-Bericht",
            ));
        }
        Ok(())
    }

    pub(crate) fn finish(self, tree: Option<SizeNode>) -> io::Result<ScanOutcome> {
        self.validate()?;
        if tree.is_some() != (self.shape.nodes > 0)
            || tree
                .as_ref()
                .is_some_and(|tree| tree.size != self.progress.bytes)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Analyse-Zähler und Baum stimmen nicht überein",
            ));
        }
        Ok(ScanOutcome {
            tree,
            status: self.status,
            issues: self.issues,
            suppressed_issues: self.suppressed_issues,
            permission_denied: self.permission_denied,
            notes: self.notes,
            aggregated_files: self.aggregated_files,
            // The wire report has no field for them: a host sends protected
            // omissions as one of its notes (`protected_note`).
            protected: Vec::new(),
        })
    }
}
