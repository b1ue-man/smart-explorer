use super::protected::MAX_PROTECTED_AREAS;
use crate::analytics::{
    tree_transfer::TreeShape, PlatformFigures, ProtectedOmission, ScanIssue, ScanOutcome,
    ScanSnapshot, ScanStatus, SizeNode, VolumeUsage,
};
use serde::{Deserialize, Serialize};
use std::io;

/// Installed apps a report may carry (Android lists a few hundred).
const MAX_PLATFORM_APPS: usize = 4096;
/// Text of protected areas and of the platform figures, each.
const MAX_FIGURE_TEXT_BYTES: usize = 256 * 1024;

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
    /// Additive (RV1): capacity of the analysed volume, measured by the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) volume: Option<VolumeUsage>,
    /// Additive (RV1): Android's figures of an Android host for that volume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) platform: Option<PlatformFigures>,
    /// Additive (RV1): protected omissions, structured. Older peers know
    /// them only from the note a host adds (`protected_note`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) protected: Vec<ProtectedOmission>,
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
            volume: outcome.volume,
            platform: outcome.platform.take(),
            protected: std::mem::take(&mut outcome.protected),
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
        let protected_bytes: usize = self.protected.iter().map(|area| area.area.len()).sum();
        let platform_apps = self
            .platform
            .as_ref()
            .map_or(&[][..], |platform| platform.apps.as_slice());
        let platform_bytes: usize = self
            .platform
            .iter()
            .flat_map(|platform| platform.app_data.iter().flatten())
            .map(String::len)
            .chain(
                platform_apps
                    .iter()
                    .map(|app| app.package.len() + app.label.len()),
            )
            .sum();
        if has_tree != (self.shape.nodes > 0)
            || self.issues.len() > 64
            || self.notes.len() > 16
            || diagnostic_bytes > 512 * 1024
            || self.protected.len() > MAX_PROTECTED_AREAS
            || protected_bytes > MAX_FIGURE_TEXT_BYTES
            || platform_apps.len() > MAX_PLATFORM_APPS
            || platform_bytes > MAX_FIGURE_TEXT_BYTES
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
            // Hosts before RV1 send protected omissions only as a note.
            protected: self.protected,
            volume: self.volume,
            platform: self.platform,
        })
    }
}
