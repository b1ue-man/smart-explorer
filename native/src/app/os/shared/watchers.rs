use super::*;

impl App {
    /// One-pass batched mutation. Collects only the affected paths instead of
    /// cloning the whole index on every remove/rename burst.
    pub(in crate::app) fn apply_batched_changes(
        &mut self,
        additions: &[String],
        remove_subtrees: &[String],
        rename_subtrees: &[(String, String)],
    ) -> bool {
        if additions.is_empty() && remove_subtrees.is_empty() && rename_subtrees.is_empty() {
            return false;
        }

        let mut dirty = false;

        if !remove_subtrees.is_empty() || !rename_subtrees.is_empty() {
            let remove_prefixes: Vec<String> =
                remove_subtrees.iter().map(|p| format!("{}/", p)).collect();
            let rename_prefixes: Vec<(String, String)> = rename_subtrees
                .iter()
                .map(|(old, new)| (format!("{}/", old), format!("{}/", new)))
                .collect();
            let remove_exact: std::collections::HashSet<&str> =
                remove_subtrees.iter().map(|s| s.as_str()).collect();

            let mut removes_to_apply: Vec<String> = Vec::new();
            let mut renames_to_apply: Vec<(String, String)> = Vec::new();

            for p in self.folder_index.iter() {
                if remove_exact.contains(p.as_str())
                    || remove_prefixes
                        .iter()
                        .any(|pref| p.starts_with(pref.as_str()))
                {
                    removes_to_apply.push(p.clone());
                    continue;
                }
                let mut renamed: Option<String> = None;
                for (old, new) in rename_subtrees {
                    if p == old {
                        renamed = Some(new.clone());
                        break;
                    }
                }
                if renamed.is_none() {
                    for (old_pref, new_pref) in &rename_prefixes {
                        if p.starts_with(old_pref.as_str()) {
                            renamed = Some(format!("{}{}", new_pref, &p[old_pref.len()..]));
                            break;
                        }
                    }
                }
                if let Some(r) = renamed {
                    renames_to_apply.push((p.clone(), r));
                }
            }

            for r in &removes_to_apply {
                if self.folder_index.remove(r) {
                    dirty = true;
                }
            }
            for (old, new) in &renames_to_apply {
                self.folder_index.remove(old);
                dirty = true;
                if !crate::folder_index::path_has_skipped_segment(new) {
                    self.folder_index.insert(new.clone());
                }
            }
        }

        for p in additions {
            if self.folder_index.insert(p.clone()) {
                dirty = true;
            }
        }
        dirty
    }
}
