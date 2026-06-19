use std::collections::HashSet;
use std::path::Path;

use super::filters::path_has_skipped_segment;
use super::model::FolderIndex;

impl FolderIndex {
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut buf = String::with_capacity(self.paths.len() * 50);
        for p in &self.paths {
            buf.push_str(p);
            buf.push('\n');
        }
        std::fs::write(path, buf)
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        // While loading, drop any entries that contain a skip-matching segment
        // anywhere in their path. This cleans up legacy indices built before
        // the filter existed - no rebuild needed.
        let paths: HashSet<String> = content
            .lines()
            .filter(|l| !l.is_empty())
            .filter(|l| !path_has_skipped_segment(l))
            .map(|l| l.to_string())
            .collect();
        let built_at = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        Ok(Self { paths, built_at })
    }
}
